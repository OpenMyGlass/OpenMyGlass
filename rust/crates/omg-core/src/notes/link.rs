//! An async loop that runs a peer over an `omg-transport` link.
//!
//! Optional glue, for frontends that let the core own the link (the CLI, the simulated
//! Glass). An app that owns its socket can call the driver directly instead.
//!
//! Reading and writing run concurrently, because a transfer has both sides sending at once.
//! With a loop that only reads between writes, two peers can each block on a full socket
//! buffer waiting for the other to read, and small Bluetooth buffers make that likely.
//! Handlers therefore never wait for the network: they return frames, which go to a queue.

use std::{panic::AssertUnwindSafe, sync::Arc, time::Duration};

use futures::{SinkExt, StreamExt};
use omg_protocol::{Bye, Frame, Pong};
use omg_transport::{Link, Stream, TransportKind};
use tokio::sync::mpsc;
use tracing::{debug, warn};

use super::{
    driver::{CompanionNotes, SpeechJob, SpeechOutcome},
    receiver::ReceiverEvent,
    speech::{SpeechToText, TextToSpeech},
    LinkProfile,
};
use crate::{now_ms, CoreError};

/// How often handlers get a tick.
const TICK: Duration = Duration::from_millis(250);
/// How long frames already queued get to reach the peer once a handler is done.
const FLUSH_TIMEOUT: Duration = Duration::from_secs(5);

impl From<TransportKind> for LinkProfile {
    fn from(kind: TransportKind) -> Self {
        match kind {
            TransportKind::Ble => LinkProfile::constrained(),
            TransportKind::BtRfcomm => LinkProfile::moderate(),
            TransportKind::WifiTcp => LinkProfile::fast(),
        }
    }
}

/// Whether the link should stay up.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Flow {
    /// Keep going.
    Continue,
    /// Send what is queued, then close.
    Close,
}

/// One side's logic on a running link. Calls never wait for the network: frames pushed to
/// `out` are sent in order, behind the scenes.
pub trait LinkHandler {
    /// Work finished elsewhere, delivered through the channel given to [`run_link`].
    type Event;
    /// The link is up.
    fn start(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<(), CoreError>;
    /// A frame arrived.
    fn on_frame(&mut self, frame: Frame, now: u64, out: &mut Vec<Frame>)
        -> Result<Flow, CoreError>;
    /// Time passed.
    fn on_tick(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<Flow, CoreError>;
    /// Work finished elsewhere.
    fn on_event(
        &mut self,
        event: Self::Event,
        now: u64,
        out: &mut Vec<Frame>,
    ) -> Result<Flow, CoreError>;
    /// The link is gone, whatever the reason. Always called.
    fn finish(&mut self, now: u64) -> Result<(), CoreError>;
}

/// Run `handler` over `link` until either side ends it or the link fails.
pub async fn run_link<S: Stream, H: LinkHandler>(
    link: Link<S>,
    handler: &mut H,
    mut events: mpsc::UnboundedReceiver<H::Event>,
) -> Result<(), CoreError> {
    let (mut sink, mut stream) = link.split();
    let (out_tx, mut out_rx) = mpsc::unbounded_channel::<Frame>();

    let mut writer = Box::pin(async move {
        while let Some(frame) = out_rx.recv().await {
            sink.send(frame).await?;
        }
        Ok::<(), CoreError>(())
    });

    let logic = async {
        // Moved in, so the queue's sender goes when this future does.
        let out_tx = out_tx;
        let send_all = |frames: &mut Vec<Frame>| {
            for frame in frames.drain(..) {
                // Fails only once the writer has already stopped with its own error.
                let _ = out_tx.send(frame);
            }
        };
        let mut out = Vec::new();
        handler.start(now_ms(), &mut out)?;
        send_all(&mut out);

        let mut tick = tokio::time::interval(TICK);
        loop {
            let flow = tokio::select! {
                frame = stream.next() => match frame {
                    None => return Ok::<(), CoreError>(()),
                    Some(Err(e)) => return Err(e.into()),
                    Some(Ok(Frame::Bye(b))) => {
                        debug!(reason = %b.reason, "peer said bye");
                        return Ok(());
                    }
                    Some(Ok(Frame::Ping(p))) => {
                        out.push(Frame::Pong(Pong { token: p.token, unix_ms: now_ms() }));
                        Flow::Continue
                    }
                    Some(Ok(frame)) => handler.on_frame(frame, now_ms(), &mut out)?,
                },
                _ = tick.tick() => handler.on_tick(now_ms(), &mut out)?,
                Some(event) = events.recv() => handler.on_event(event, now_ms(), &mut out)?,
            };
            send_all(&mut out);
            if flow == Flow::Close {
                return Ok(());
            }
        }
    };

    let result = {
        tokio::pin!(logic);
        tokio::select! {
            result = &mut logic => result,
            // The writer can end first only on an error: the logic holds the queue open.
            result = &mut writer => result,
        }
    };
    if result.is_ok() {
        // The logic is gone, so the writer stops once what is queued has been sent.
        match tokio::time::timeout(FLUSH_TIMEOUT, &mut writer).await {
            Ok(Ok(())) => {}
            Ok(Err(e)) => debug!(error = %e, "link failed while flushing"),
            Err(_) => debug!("link did not drain in time"),
        }
    }
    drop(writer);
    let finished = handler.finish(now_ms());
    result.and(finished)
}

/// What a frontend sees during [`run_companion`].
#[derive(Debug)]
pub enum CompanionUpdate<'a> {
    /// Something happened with a note.
    Note(&'a ReceiverEvent),
    /// A frame that has nothing to do with notes, e.g. a battery push.
    Other(&'a Frame),
}

/// Options for [`run_companion`].
#[derive(Debug, Clone, Copy)]
pub struct CompanionRunOptions {
    /// How to pull over this link.
    pub profile: LinkProfile,
    /// Say goodbye once there has been nothing to do for this long. `None` stays up until
    /// the peer leaves.
    pub close_when_idle_for: Option<Duration>,
}

/// Run the companion's note sync over an authenticated link until it ends. Call only if
/// the peer advertised the notes capability. Speech jobs run on blocking threads.
pub async fn run_companion<S, F>(
    link: Link<S>,
    notes: &mut CompanionNotes,
    stt: Arc<dyn SpeechToText>,
    tts: Arc<dyn TextToSpeech>,
    options: CompanionRunOptions,
    observer: F,
) -> Result<(), CoreError>
where
    S: Stream,
    F: FnMut(CompanionUpdate<'_>),
{
    let (done_tx, done_rx) = mpsc::unbounded_channel();
    let mut handler = CompanionHandler {
        notes,
        stt,
        tts,
        options,
        observer,
        done_tx,
        idle_since: None,
    };
    run_link(link, &mut handler, done_rx).await
}

struct CompanionHandler<'a, F> {
    notes: &'a mut CompanionNotes,
    stt: Arc<dyn SpeechToText>,
    tts: Arc<dyn TextToSpeech>,
    options: CompanionRunOptions,
    observer: F,
    done_tx: mpsc::UnboundedSender<SpeechOutcome>,
    idle_since: Option<u64>,
}

impl<F: FnMut(CompanionUpdate<'_>)> CompanionHandler<'_, F> {
    /// Hand over what the driver produced: speech jobs to threads, events to the observer.
    fn drain(&mut self) {
        for job in self.notes.take_speech_jobs() {
            let (stt, tts, done) = (self.stt.clone(), self.tts.clone(), self.done_tx.clone());
            tokio::task::spawn_blocking(move || {
                let outcome = run_guarded(&job, stt.as_ref(), tts.as_ref());
                // The link may be gone. The job then simply runs again on the next one.
                let _ = done.send(outcome);
            });
        }
        for event in self.notes.take_events() {
            (self.observer)(CompanionUpdate::Note(&event));
        }
    }
}

/// An engine is foreign code: a panic in it must not leave the note waiting forever.
fn run_guarded(job: &SpeechJob, stt: &dyn SpeechToText, tts: &dyn TextToSpeech) -> SpeechOutcome {
    std::panic::catch_unwind(AssertUnwindSafe(|| job.run(stt, tts))).unwrap_or_else(|_| {
        warn!("a speech engine panicked");
        job.failed("the engine panicked")
    })
}

impl<F: FnMut(CompanionUpdate<'_>)> LinkHandler for CompanionHandler<'_, F> {
    type Event = SpeechOutcome;

    fn start(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<(), CoreError> {
        out.extend(self.notes.link_up(self.options.profile, now)?);
        self.drain();
        Ok(())
    }

    fn on_frame(
        &mut self,
        frame: Frame,
        now: u64,
        out: &mut Vec<Frame>,
    ) -> Result<Flow, CoreError> {
        if !is_note_frame(&frame) {
            (self.observer)(CompanionUpdate::Other(&frame));
            return Ok(Flow::Continue);
        }
        out.extend(self.notes.on_frame(frame, now)?);
        self.drain();
        Ok(Flow::Continue)
    }

    fn on_tick(&mut self, now: u64, out: &mut Vec<Frame>) -> Result<Flow, CoreError> {
        out.extend(self.notes.tick(now)?);
        self.drain();
        let Some(grace) = self.options.close_when_idle_for else {
            return Ok(Flow::Continue);
        };
        if !self.notes.machine().is_idle() {
            self.idle_since = None;
            return Ok(Flow::Continue);
        }
        let since = *self.idle_since.get_or_insert(now);
        if now.saturating_sub(since) < grace.as_millis() as u64 {
            return Ok(Flow::Continue);
        }
        out.push(Frame::Bye(Bye {
            reason: "note sync finished".into(),
        }));
        Ok(Flow::Close)
    }

    fn on_event(
        &mut self,
        event: SpeechOutcome,
        now: u64,
        out: &mut Vec<Frame>,
    ) -> Result<Flow, CoreError> {
        out.extend(self.notes.complete_speech(event, now)?);
        self.drain();
        Ok(Flow::Continue)
    }

    fn finish(&mut self, now: u64) -> Result<(), CoreError> {
        let result = self.notes.link_down(now);
        self.drain();
        result
    }
}

/// Whether a frame belongs to note sync.
pub fn is_note_frame(frame: &Frame) -> bool {
    frame.wire_name().starts_with("Note")
}

#[cfg(test)]
mod tests {
    use super::*;
    use omg_protocol::{NoteSyncStatus, Ping};

    #[test]
    fn note_frames_are_told_apart_from_notification_frames() {
        assert!(is_note_frame(&Frame::NoteSyncStatus(NoteSyncStatus {
            pending_notes: 0,
            pending_bytes: 0,
            sending: false,
        })));
        assert!(!is_note_frame(&Frame::NotificationRemoved(
            omg_protocol::NotificationRemoved { key: "k".into() }
        )));
        assert!(!is_note_frame(&Frame::Ping(Ping {
            token: 0,
            unix_ms: 0
        })));
    }

    #[test]
    fn transports_map_to_a_profile() {
        assert_eq!(
            LinkProfile::from(TransportKind::Ble),
            LinkProfile::constrained()
        );
        assert_eq!(
            LinkProfile::from(TransportKind::BtRfcomm),
            LinkProfile::moderate()
        );
        assert_eq!(
            LinkProfile::from(TransportKind::WifiTcp),
            LinkProfile::fast()
        );
    }
}
