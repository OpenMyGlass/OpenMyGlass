package org.openmyglass.glass.bridge;

import java.io.Closeable;
import java.io.IOException;
import java.net.ServerSocket;
import java.net.Socket;

import org.openmyglass.protocol.v1.Bridge;

/**
 * Glass side of the OpenMyGlass bridge (protocol openmyglass.v1) over TCP. One session at a time:
 * a new connection replaces the active one, because it means the phone reconnected.
 * The phone speaks first; we answer its Hello. Blocking I/O on two threads (accept, session).
 */
public final class BridgeServer implements Closeable {

    public interface Sender {
        void send(Bridge.Frame frame) throws IOException;
    }

    /** Called on the session thread; onSessionEnded always follows onSessionStarted. */
    public interface FrameHandler {
        void onSessionStarted(Sender sender);
        void onFrame(Bridge.Frame frame);
        void onSessionEnded();
    }

    public static final int PROTOCOL_VERSION = 1;

    private final int port;
    private final FrameHandler handler;
    private volatile ServerSocket server;
    private volatile boolean closed;
    private Socket activeSocket;
    private Thread activeThread;

    public BridgeServer(int port, FrameHandler handler) {
        this.port = port;
        this.handler = handler;
    }

    /** Starts listening and returns the bound port. */
    public int start() {
        try {
            server = new ServerSocket(port);
        } catch (IOException e) {
            throw new IllegalStateException("cannot listen on port " + port, e);
        }
        Thread accept = new Thread(new Runnable() {
            @Override public void run() { acceptLoop(); }
        }, "bridge-accept");
        accept.setDaemon(true);
        accept.start();
        return server.getLocalPort();
    }

    @Override
    public void close() {
        closed = true;
        closeQuietly(server);
        replaceSession(null, null);
    }

    private void acceptLoop() {
        while (!closed) {
            final Socket socket;
            try {
                socket = server.accept();
            } catch (IOException e) {
                return;
            }
            Thread session = new Thread(new Runnable() {
                @Override public void run() { serve(socket); }
            }, "bridge-session");
            session.setDaemon(true);
            replaceSession(socket, session);
            session.start();
        }
    }

    /** Ends the active session (its onSessionEnded runs before the new session can start). */
    private synchronized void replaceSession(Socket socket, Thread thread) {
        if (activeSocket != null) {
            closeQuietly(activeSocket);
            try {
                activeThread.join(2_000);
            } catch (InterruptedException e) {
                Thread.currentThread().interrupt();
            }
        }
        activeSocket = socket;
        activeThread = thread;
    }

    private void serve(final Socket socket) {
        boolean started = false;
        try {
            Bridge.Frame first = read(socket);
            if (first == null || !first.hasHello()) return;
            final Object writeLock = new Object();
            Sender sender = new Sender() {
                @Override public void send(Bridge.Frame frame) throws IOException {
                    synchronized (writeLock) {
                        Framing.write(socket.getOutputStream(), frame.toByteArray());
                    }
                }
            };
            sender.send(Bridge.Frame.newBuilder().setHello(Bridge.Hello.newBuilder()
                    .setProtocolVersion(PROTOCOL_VERSION).setAppVersion("glass-bridge")
                    .addCapabilities("nav").addCapabilities("icons")).build());
            handler.onSessionStarted(sender);
            started = true;
            Bridge.Frame frame;
            while ((frame = read(socket)) != null) handler.onFrame(frame);
        } catch (IOException e) {
            // malformed frame, link dropped or session replaced: the session ends, the server keeps accepting
        } finally {
            closeQuietly(socket);
            if (started) handler.onSessionEnded();
        }
    }

    private static Bridge.Frame read(Socket socket) throws IOException {
        byte[] payload = Framing.read(socket.getInputStream());
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
