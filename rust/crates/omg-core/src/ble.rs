//! What a phone needs to carry the link over BLE: the GATT service and paced writes. The radio
//! calls stay in each app, in Kotlin and in Swift; this is the part they would otherwise each
//! write. The Glass end is `omg-bled` in glass-services, and `docs/companion-mobile.md` has the
//! whole picture.
//!
//! The stream is the same bytes as on TCP, with no header: each write or notification is the next
//! run. A lost run shows as a tag failure in the encrypted link, which ends it.
//!
//! # Why pace
//!
//! A phone's stack takes writes without response much faster than the radio sends them. Measured
//! on a Redmi Note 8T against the Glass: unpaced, writes queued for up to 13 s inside Android, ahead
//! of anything sent later. At 3 kB/s the typical delay was 10 ms, with zero loss, apart from stalls
//! of up to 2.7 s in some runs that pacing does not explain.

use std::collections::VecDeque;

/// The service, on the Glass. Spells `omg-ble-link-v1` and an index.
pub const SERVICE_UUID: &str = "6f6d672d-626c-652d-6c69-6e6b2d763100";
/// Phone to Glass: Write Without Response.
pub const RX_UUID: &str = "6f6d672d-626c-652d-6c69-6e6b2d763101";
/// Glass to phone: Notify.
pub const TX_UUID: &str = "6f6d672d-626c-652d-6c69-6e6b2d763102";
/// The standard Client Characteristic Configuration Descriptor.
pub const CCCD_UUID: &str = "00002902-0000-1000-8000-00805f9b34fb";
/// The ATT MTU to ask for. The Glass answers with at most this.
pub const REQUESTED_MTU: u16 = 247;
/// Bytes a write can carry at an MTU.
pub const fn run_len(mtu: u16) -> usize {
    mtu.saturating_sub(3) as usize
}
/// The default bytes per second from phone to Glass. Below the 5.4 to 6.1 kB/s the link carried
/// unpaced, at the speed where the queue stayed flat.
pub const WRITE_RATE: u32 = 3_000;
/// What a quiet link may send at once, so that a short frame does not wait.
const BURST_BYTES: u32 = 488;

/// What the app does next with its writes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WriteStep {
    /// One write without response. Ask again when the stack has taken it.
    Write(Vec<u8>),
    /// Nothing may go for this long, in milliseconds.
    Wait(u32),
    /// Everything has been handed over.
    Idle,
}

/// Splits outgoing bytes into runs that fit the MTU, and spaces them. Pure: it gets the time.
#[derive(Debug)]
pub struct WritePacer {
    queue: VecDeque<u8>,
    mtu: u16,
    rate: u32,
    /// Bytes that may go now, scaled by 1000 so that short steps do not round to nothing.
    allowance: u64,
    last_ms: Option<u64>,
}

impl WritePacer {
    /// At the default rate, assuming the MTU every link starts with until it is told better.
    pub fn new() -> WritePacer {
        WritePacer::with_rate(WRITE_RATE)
    }

    /// `rate` is in bytes per second, at least 1.
    pub fn with_rate(rate: u32) -> WritePacer {
        WritePacer {
            queue: VecDeque::new(),
            mtu: 23,
            rate: rate.max(1),
            allowance: u64::from(BURST_BYTES) * 1000,
            last_ms: None,
        }
    }

    /// The negotiated ATT MTU.
    pub fn set_mtu(&mut self, mtu: u16) {
        self.mtu = mtu.max(23);
    }

    /// Bytes to send, after those already here.
    pub fn push(&mut self, bytes: &[u8]) {
        self.queue.extend(bytes);
    }

    /// Bytes not handed over yet.
    pub fn queued(&self) -> usize {
        self.queue.len()
    }

    /// What to do at `now_ms`, on any clock that does not go backwards.
    pub fn next_step(&mut self, now_ms: u64) -> WriteStep {
        let elapsed = self.last_ms.map_or(0, |last| now_ms.saturating_sub(last));
        self.last_ms = Some(now_ms);
        self.allowance = (self.allowance + elapsed * u64::from(self.rate))
            .min(u64::from(BURST_BYTES.max(self.mtu as u32)) * 1000);
        if self.queue.is_empty() {
            return WriteStep::Idle;
        }
        let n = self.queue.len().min(run_len(self.mtu));
        let cost = n as u64 * 1000;
        if self.allowance < cost {
            let wait = (cost - self.allowance).div_ceil(u64::from(self.rate));
            return WriteStep::Wait(wait.min(u64::from(u32::MAX)) as u32);
        }
        self.allowance -= cost;
        WriteStep::Write(self.queue.drain(..n).collect())
    }
}

impl Default for WritePacer {
    fn default() -> Self {
        WritePacer::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Runs the pacer on a clock of its own. Returns each run and when it went.
    fn drain(p: &mut WritePacer, start: u64) -> Vec<(u64, Vec<u8>)> {
        let mut now = start;
        let mut out = Vec::new();
        loop {
            match p.next_step(now) {
                WriteStep::Idle => return out,
                WriteStep::Wait(ms) => {
                    assert!(ms > 0, "waiting no time would spin");
                    now += u64::from(ms);
                }
                WriteStep::Write(run) => out.push((now, run)),
            }
        }
    }

    #[test]
    fn runs_fit_the_mtu_and_nothing_is_lost_or_moved() {
        let bytes: Vec<u8> = (0..10_000).map(|i| (i % 251) as u8).collect();
        for mtu in [23, 100, 247] {
            let mut p = WritePacer::new();
            p.set_mtu(mtu);
            p.push(&bytes[..4000]);
            p.push(&bytes[4000..]);
            let runs = drain(&mut p, 0);
            assert!(runs.iter().all(|(_, r)| r.len() <= mtu as usize - 3));
            assert_eq!(runs.concat_bytes(), bytes);
            assert_eq!(p.queued(), 0);
        }
    }

    trait Concat {
        fn concat_bytes(&self) -> Vec<u8>;
    }

    impl Concat for Vec<(u64, Vec<u8>)> {
        fn concat_bytes(&self) -> Vec<u8> {
            self.iter().flat_map(|(_, r)| r.iter().copied()).collect()
        }
    }

    #[test]
    fn the_rate_holds_over_a_long_transfer() {
        let mut p = WritePacer::new();
        p.set_mtu(247);
        p.push(&vec![0; 30_000]);
        let runs = drain(&mut p, 1000);
        let took = runs.last().unwrap().0 - 1000;
        // 30 kB at 3 kB/s, less the burst it starts with.
        assert!((9_700..=10_000).contains(&took), "{took} ms");
    }

    #[test]
    fn a_short_frame_on_a_quiet_link_goes_at_once() {
        let mut p = WritePacer::new();
        p.set_mtu(247);
        p.push(&[1; 100]);
        assert_eq!(p.next_step(5000), WriteStep::Write(vec![1; 100]));
        assert_eq!(p.next_step(5000), WriteStep::Idle);
    }

    #[test]
    fn a_quiet_time_does_not_save_up_a_flood() {
        let mut p = WritePacer::new();
        p.set_mtu(247);
        assert_eq!(p.next_step(0), WriteStep::Idle);
        p.push(&vec![0; 5000]);
        let runs = drain(&mut p, 3_600_000);
        // All of it at one instant would queue in the stack, which is what this is for.
        let at_once = runs.iter().filter(|(t, _)| *t == 3_600_000).count();
        assert!(at_once <= 2, "{at_once} runs at once");
    }

    #[test]
    fn a_clock_going_back_does_not_break_it() {
        let mut p = WritePacer::with_rate(1000);
        p.set_mtu(247);
        p.push(&[0; 1000]);
        let _ = p.next_step(1000);
        let _ = p.next_step(500);
        assert!(!drain(&mut p, 500).is_empty());
    }

    #[test]
    fn the_uuids_are_one_family() {
        assert_eq!(&SERVICE_UUID[..34], &RX_UUID[..34]);
        assert_eq!(&SERVICE_UUID[..34], &TX_UUID[..34]);
        assert_eq!(run_len(REQUESTED_MTU), 244);
    }
}
