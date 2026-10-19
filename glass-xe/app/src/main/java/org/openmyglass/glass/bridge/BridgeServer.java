package org.openmyglass.glass.bridge;

import java.io.Closeable;
import java.io.IOException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.List;
import java.util.Timer;
import java.util.TimerTask;

import org.openmyglass.protocol.v1.Bridge;

/**
 * Glass side of the OpenMyGlass bridge (protocol openmyglass.v1). Accepts on one or more {@link ServerChannel}s
 * (TCP, Bluetooth RFCOMM) with one session at a time across all of them: a new connection replaces the active
 * one, because it means the phone reconnected. The phone speaks first; we answer its Hello.
 * Blocking I/O: one accept thread per channel, one thread per session.
 */
public final class BridgeServer implements Closeable {

    public interface Sender {
        void send(Bridge.Frame frame) throws IOException;
    }

    /** Called on the session thread; onSessionEnded always follows onSessionStarted. */
    public interface FrameHandler {
        void onSessionStarted(Sender sender);
        void onFrame(Bridge.Frame frame);
        /** {@code sender} identifies the session that ended: a late end of a replaced session can be ignored. */
        void onSessionEnded(Sender sender);
    }

    public static final int PROTOCOL_VERSION = 1;
    public static final int DEFAULT_HELLO_TIMEOUT_MS = 5_000;
    public static final int DEFAULT_REOPEN_DELAY_MS = 5_000;
    private static final int MAX_REOPEN_DELAY_MS = 60_000;

    private final List<ServerChannel> channels;
    private final FrameHandler handler;
    private final int helloTimeoutMs;
    private final int reopenDelayMs;
    private final Timer helloTimer = new Timer("bridge-hello-timeout", true);
    private volatile boolean closed;
    private ServerChannel.Connection activeConnection;
    private Thread activeThread;

    public BridgeServer(int port, FrameHandler handler) {
        this(port, handler, DEFAULT_HELLO_TIMEOUT_MS);
    }

    public BridgeServer(int port, FrameHandler handler, int helloTimeoutMs) {
        this(Collections.<ServerChannel>singletonList(new TcpServerChannel(port)), handler, helloTimeoutMs);
    }

    public BridgeServer(List<ServerChannel> channels, FrameHandler handler, int helloTimeoutMs) {
        this(channels, handler, helloTimeoutMs, DEFAULT_REOPEN_DELAY_MS);
    }

    /**
     * A connection becomes the active session only after it sends a valid Hello within {@code helloTimeoutMs}:
     * a stray connection (e.g. a port scan on the same Wi-Fi) cannot drop the phone's session. A channel whose
     * accept fails (Bluetooth turned off) is reopened after {@code reopenDelayMs}, doubling up to 60 s.
     */
    public BridgeServer(List<ServerChannel> channels, FrameHandler handler, int helloTimeoutMs, int reopenDelayMs) {
        this.channels = new ArrayList<ServerChannel>(channels);
        this.handler = handler;
        this.helloTimeoutMs = helloTimeoutMs;
        this.reopenDelayMs = reopenDelayMs;
    }

    /**
     * Starts listening on every channel and returns the port of the first TCP channel (-1 if none or not bound yet).
     * A channel that cannot open now (Bluetooth off, port still held) is retried in the background.
     */
    public int start() {
        int port = -1;
        for (final ServerChannel channel : channels) {
            boolean opened = true;
            try {
                channel.open();
            } catch (IOException e) {
                opened = false;   // Bluetooth off, or a TCP port still held: retried in the background
            }
            if (port < 0 && channel instanceof TcpServerChannel) port = ((TcpServerChannel) channel).localPort();
            final boolean ready = opened;
            Thread accept = new Thread(new Runnable() {
                @Override public void run() { acceptLoop(channel, ready); }
            }, "bridge-accept");
            accept.setDaemon(true);
            accept.start();
        }
        return port;
    }

    @Override
    public void close() {
        closed = true;
        for (ServerChannel channel : channels) channel.close();
        helloTimer.cancel();
        replaceSession(null, null);
    }

    private void acceptLoop(ServerChannel channel, boolean opened) {
        int delay = reopenDelayMs;
        while (!closed) {
            if (!opened) {
                try {
                    Thread.sleep(delay);
                    if (closed) return;
                    channel.open();
                    if (closed) {   // close() ran while we were reopening
                        channel.close();
                        return;
                    }
                    opened = true;
                } catch (InterruptedException e) {
                    return;
                } catch (IOException e) {
                    delay = Math.min(delay * 2, MAX_REOPEN_DELAY_MS);
                    continue;
                }
            }
            final ServerChannel.Connection connection;
            try {
                connection = channel.accept();
            } catch (IOException e) {
                if (closed) return;
                channel.close();
                opened = false;
                continue;
            }
            delay = reopenDelayMs;
            Thread session = new Thread(new Runnable() {
                @Override public void run() { serve(connection); }
            }, "bridge-session");
            session.setDaemon(true);
            session.start();
        }
    }

    /**
     * Ends the active session (its onSessionEnded runs before the new session can start). Returns false, and closes
     * {@code connection}, if the server is already closed.
     */
    private synchronized boolean replaceSession(ServerChannel.Connection connection, Thread thread) {
        if (closed && connection != null) {
            closeQuietly(connection);
            return false;
        }
        if (activeConnection != null) {
            closeQuietly(activeConnection);
            try {
                activeThread.join(2_000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        activeConnection = connection;
        activeThread = thread;
        return true;
    }

    private void serve(final ServerChannel.Connection connection) {
        Sender started = null;
        // RFCOMM sockets have no read timeout: close the connection if no Hello arrives in time
        TimerTask helloDeadline = new TimerTask() {
            @Override public void run() { closeQuietly(connection); }
        };
        try {
            try {
                helloTimer.schedule(helloDeadline, helloTimeoutMs);
            } catch (IllegalStateException e) {
                return;   // server closed
            }
            Bridge.Frame first = read(connection);
            if (first == null || !first.hasHello() || !helloDeadline.cancel()) return;
            if (first.getHello().getProtocolVersion() != PROTOCOL_VERSION) return;   // incompatible phone
            if (!replaceSession(connection, Thread.currentThread())) return;
            final Object writeLock = new Object();
            Sender sender = new Sender() {
                @Override public void send(Bridge.Frame frame) throws IOException {
                    synchronized (writeLock) {
                        Framing.write(connection.output(), frame.toByteArray());
                    }
                }
            };
            sender.send(Bridge.Frame.newBuilder().setHello(Bridge.Hello.newBuilder()
                    .setProtocolVersion(PROTOCOL_VERSION).setAppVersion("glass-bridge")
                    .addCapabilities("nav").addCapabilities("icons")).build());
            handler.onSessionStarted(sender);
            started = sender;
            Bridge.Frame frame;
            while ((frame = read(connection)) != null) handler.onFrame(frame);
        } catch (IOException e) {
            // malformed frame, link dropped or session replaced: the session ends, the server keeps accepting
        } finally {
            helloDeadline.cancel();
            closeQuietly(connection);
            if (started != null) handler.onSessionEnded(started);
        }
    }

    private static Bridge.Frame read(ServerChannel.Connection connection) throws IOException {
        byte[] payload = Framing.read(connection.input());
        return payload == null ? null : Bridge.Frame.parseFrom(payload);
    }

    private static void closeQuietly(Closeable c) {
        if (c == null) return;
        try {
            c.close();
        } catch (IOException ignored) {
            // nothing to do
        }
    }
}
