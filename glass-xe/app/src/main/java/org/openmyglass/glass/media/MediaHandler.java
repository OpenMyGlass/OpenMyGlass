package org.openmyglass.glass.media;

import java.io.File;
import java.io.IOException;
import java.io.RandomAccessFile;
import java.security.SecureRandom;
import java.util.List;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.protocol.v1.Bridge;

/**
 * Glass side of photo/video sync over the bridge: reports charging and Wi-Fi, lists media, and opens a
 * {@link MediaHttpServer} with a fresh token for each pull (only on Wi-Fi), closing it when the phone is done or
 * the session ends.
 */
public final class MediaHandler implements BridgeServer.FrameHandler {

    public interface Status {
        boolean charging();
        boolean wifi();
        String ip();
    }

    private static final int IDLE_MS = 120_000;
    /** Keeps a chunk frame below the bridge's 64 KiB cap. */
    private static final int CHUNK_MAX = 48 * 1024;

    private final MediaIndex index;
    private final Status status;
    private final SecureRandom random = new SecureRandom();
    private BridgeServer.Sender sender;
    private MediaHttpServer server;

    public MediaHandler(MediaIndex index, Status status) {
        this.index = index;
        this.status = status;
    }

    @Override
    public synchronized void onSessionStarted(BridgeServer.Sender sender) {
        this.sender = sender;
        sendStatus();
    }

    @Override
    public synchronized void onFrame(Bridge.Frame frame) {
        if (frame.hasMediaListRequest()) {
            sendList();
        } else if (frame.hasMediaPullStart()) {
            closeServer();
            if (!status.wifi() || status.ip().length() == 0) {
                send(Bridge.Frame.newBuilder().setMediaServerUnavailable(
                        Bridge.MediaServerUnavailable.newBuilder().setReason("Glass is not on Wi-Fi")).build());
                return;
            }
            byte[] t = new byte[16];
            random.nextBytes(t);
            StringBuilder token = new StringBuilder();
            for (byte b : t) token.append(String.format("%02x", b));
            try {
                server = new MediaHttpServer(index, token.toString(), IDLE_MS);
                int port = server.start();
                send(Bridge.Frame.newBuilder().setMediaServer(Bridge.MediaServer.newBuilder()
                        .setIp(status.ip()).setPort(port).setToken(token.toString())).build());
            } catch (IOException e) {
                server = null;
                send(Bridge.Frame.newBuilder().setMediaServerUnavailable(
                        Bridge.MediaServerUnavailable.newBuilder().setReason("Glass could not open the media server")).build());
            }
        } else if (frame.hasMediaPullDone()) {
            closeServer();
        } else if (frame.hasMediaChunkRequest()) {
            send(Bridge.Frame.newBuilder().setMediaChunk(chunk(frame.getMediaChunkRequest())).build());
        } else if (frame.hasMediaDelete()) {
            String id = frame.getMediaDelete().getId();
            File f = index.fileIfUnchanged(id);   // a changed file has another id: never delete what the phone did not save
            boolean ok = f != null && f.delete();
            if (ok) index.scan();
            send(Bridge.Frame.newBuilder().setMediaDeleted(Bridge.MediaDeleted.newBuilder().setId(id).setOk(ok)).build());
        }
    }

    @Override
    public synchronized void onSessionEnded(BridgeServer.Sender ended) {
        if (ended != sender) return;
        sender = null;
        closeServer();
    }

    /** A new photo or video was written (FileObserver). */
    public synchronized void onNewMedia() {
        if (sender != null) sendList();
    }

    /** Charging or Wi-Fi changed. */
    public synchronized void onStatusChanged() {
        if (sender != null) sendStatus();
    }

    /** One chunk of an indexed file (Bluetooth fallback); errors come back as an empty chunk with a reason. */
    private Bridge.MediaChunk chunk(Bridge.MediaChunkRequest r) {
        Bridge.MediaChunk.Builder c = Bridge.MediaChunk.newBuilder().setId(r.getId()).setOffset(r.getOffset());
        File f = index.fileIfUnchanged(r.getId());
        if (f == null) return c.setError("unknown or changed file").build();
        long total = f.length();
        if (r.getOffset() < 0 || r.getOffset() > total) return c.setTotal(total).setError("bad offset").build();
        int length = (int) Math.min(Math.min(r.getLength(), CHUNK_MAX), total - r.getOffset());
        try {
            byte[] data = new byte[length];
            RandomAccessFile raf = new RandomAccessFile(f, "r");
            try {
                raf.seek(r.getOffset());
                raf.readFully(data);
            } finally {
                raf.close();
            }
            return c.setData(com.google.protobuf.ByteString.copyFrom(data)).setTotal(total).setSha256(index.sha256(f)).build();
        } catch (IOException e) {
            return c.setTotal(total).setError("read failed").build();
        }
    }

    private void sendStatus() {
        send(Bridge.Frame.newBuilder().setGlassStatus(Bridge.GlassStatus.newBuilder()
                .setCharging(status.charging()).setWifi(status.wifi()).setIp(status.ip())).build());
    }

    private void sendList() {
        List<MediaIndex.Item> items = index.scan();
        Bridge.MediaList.Builder list = Bridge.MediaList.newBuilder();
        for (MediaIndex.Item i : items) {
            list.addItems(Bridge.MediaItem.newBuilder().setId(i.id).setName(i.name).setVideo(i.video)
                    .setSize(i.size).setTakenAtMs(i.takenAtMs));
        }
        send(Bridge.Frame.newBuilder().setMediaList(list).build());
    }

    private void closeServer() {
        if (server != null) server.close();
        server = null;
    }

    private void send(Bridge.Frame frame) {
        if (sender == null) return;
        try {
            sender.send(frame);
        } catch (IOException e) {
            // the session is ending
        }
    }
}
