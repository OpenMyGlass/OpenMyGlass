package org.openmyglass.glass.bridge;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.net.ServerSocket;
import java.net.Socket;

/** TCP listener on all interfaces (debug path; no authentication, see docs/glass-app.md). */
public final class TcpServerChannel implements ServerChannel {
    private final int port;
    private volatile ServerSocket server;

    public TcpServerChannel(int port) {
        this.port = port;
    }

    @Override
    public void open() throws IOException {
        // Reopening keeps the port chosen the first time (port 0 in tests)
        int bind = server != null ? server.getLocalPort() : port;
        server = new ServerSocket(bind);
    }

    /** The bound port, or -1 before {@link #open()}. */
    public int localPort() {
        ServerSocket s = server;
        return s == null ? -1 : s.getLocalPort();
    }

    @Override
    public Connection accept() throws IOException {
        final Socket socket = server.accept();
        return new Connection() {
            @Override public InputStream input() throws IOException { return socket.getInputStream(); }
            @Override public OutputStream output() throws IOException { return socket.getOutputStream(); }
            @Override public void close() throws IOException { socket.close(); }
        };
    }

    @Override
    public void close() {
        ServerSocket s = server;
        if (s != null) {
            try {
                s.close();
            } catch (IOException ignored) {
                // closing
            }
        }
    }
}
