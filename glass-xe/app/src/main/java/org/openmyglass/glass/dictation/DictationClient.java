package org.openmyglass.glass.dictation;

import java.io.IOException;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.protocol.v1.Bridge;

/**
 * Glass side of a dictated reply over the bridge: streams the audio of one request at a time and delivers the
 * phone's transcript and reply result for that request (older requests are ignored). Sends block on the bridge
 * socket: call off the main thread.
 */
public final class DictationClient implements BridgeServer.FrameHandler {

    public interface Listener {
        /** {@code error} non-empty: no usable transcript (shown to the owner). */
        void onTranscript(int requestId, String text, String error);
        void onResult(int requestId, boolean ok, String message);
        /** The phone went away while a dictation was open. */
        void onUnavailable();
    }

    private final Listener listener;
    private BridgeServer.Sender sender;
    private int lastId;
    private int current = -1;
    private int seq;

    public DictationClient(Listener listener) {
        this.listener = listener;
    }

    /** Opens a request; returns its id, or -1 if the phone is not connected (do not record then). */
    public int start(String key, int actionIndex, int sampleRate) {
        BridgeServer.Sender s;
        int id;
        synchronized (this) {
            if (sender == null) return -1;
            s = sender;
            id = ++lastId;
            current = id;
            seq = 0;
        }
        boolean ok = send(s, Bridge.Frame.newBuilder().setDictationStart(Bridge.DictationStart.newBuilder()
                .setRequestId(id).setKey(key).setActionIndex(actionIndex).setSampleRate(sampleRate)).build());
        return ok ? id : -1;
    }

    /** Streams PCM16 LE mono; false if the request is no longer current or the link failed. */
    public boolean audio(int id, byte[] pcm) {
        BridgeServer.Sender s;
        int n;
        synchronized (this) {
            if (id != current || sender == null) return false;
            s = sender;
            n = seq++;
        }
        return send(s, Bridge.Frame.newBuilder().setAudioChunk(Bridge.AudioChunk.newBuilder()
                .setRequestId(id).setSeq(n).setPcm(com.google.protobuf.ByteString.copyFrom(pcm))).build());
    }

    public void finish(int id) {
        end(id, false);
    }

    public void cancel(int id) {
        end(id, true);
        synchronized (this) {
            if (id == current) current = -1;
        }
    }

    public void confirm(int id, boolean send) {
        send(currentSender(), Bridge.Frame.newBuilder().setReplyConfirm(
                Bridge.ReplyConfirm.newBuilder().setRequestId(id).setSend(send)).build());
    }

    @Override
    public synchronized void onSessionStarted(BridgeServer.Sender sender) {
        this.sender = sender;
    }

    @Override
    public void onFrame(Bridge.Frame frame) {
        int now;
        synchronized (this) {
            now = current;
        }
        if (frame.hasTranscript() && frame.getTranscript().getRequestId() == now) {
            Bridge.Transcript t = frame.getTranscript();
            listener.onTranscript(t.getRequestId(), t.getText(), t.getError());
        } else if (frame.hasReplyResult() && frame.getReplyResult().getRequestId() == now) {
            Bridge.ReplyResult r = frame.getReplyResult();
            listener.onResult(r.getRequestId(), r.getOk(), r.getMessage());
        }
    }

    @Override
    public void onSessionEnded(BridgeServer.Sender ended) {
        boolean wasOpen;
        synchronized (this) {
            if (ended != sender) return;
            sender = null;
            wasOpen = current > 0;
            current = -1;
        }
        if (wasOpen) listener.onUnavailable();
    }

    private void end(int id, boolean cancelled) {
        BridgeServer.Sender s;
        synchronized (this) {
            if (id != current) return;
            s = sender;
        }
        send(s, Bridge.Frame.newBuilder().setDictationEnd(
                Bridge.DictationEnd.newBuilder().setRequestId(id).setCancelled(cancelled)).build());
    }

    private synchronized BridgeServer.Sender currentSender() {
        return sender;
    }

    private static boolean send(BridgeServer.Sender s, Bridge.Frame frame) {
        if (s == null) return false;
        try {
            s.send(frame);
            return true;
        } catch (IOException e) {
            return false;
        }
    }
}
