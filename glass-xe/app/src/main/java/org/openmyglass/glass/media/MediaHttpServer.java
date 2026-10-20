package org.openmyglass.glass.media;

import java.io.BufferedOutputStream;
import java.io.File;
import java.io.FileInputStream;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.io.RandomAccessFile;
import java.net.ServerSocket;
import java.net.Socket;
import java.net.SocketTimeoutException;
import java.security.MessageDigest;
import java.util.HashMap;
import java.util.Map;

/**
 * Serves indexed media to the phone over Wi-Fi during one pull: {@code GET /media/<id>?token=<t>}, with Range
 * (resume) and an {@code X-Content-SHA256} header. Requests without the pull's token get 403; only ids from
 * {@link MediaIndex} are served. The server closes itself after {@code idleMs} without a new request.
 */
public final class MediaHttpServer {
    private final MediaIndex index;
    private final byte[] token;
    private final int idleMs;
    private final Map<String, String> hashes = new HashMap<String, String>();
    private volatile ServerSocket server;

    public MediaHttpServer(MediaIndex index, String token, int idleMs) {
        this.index = index;
        this.token = token.getBytes();
        this.idleMs = idleMs;
    }

    /** Listens on an ephemeral port on all interfaces; returns the port. */
    public int start() throws IOException {
        server = new ServerSocket(0);
        server.setSoTimeout(idleMs);
        Thread accept = new Thread(new Runnable() {
            @Override public void run() { acceptLoop(); }
        }, "media-accept");
        accept.setDaemon(true);
        accept.start();
        return server.getLocalPort();
    }

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

    private void acceptLoop() {
        while (true) {
            final Socket socket;
            try {
                socket = server.accept();
            } catch (SocketTimeoutException e) {
                close();   // idle: the pull is over or abandoned
                return;
            } catch (IOException e) {
                return;
            }
            Thread t = new Thread(new Runnable() {
                @Override public void run() { handle(socket); }
            }, "media-request");
            t.setDaemon(true);
            t.start();
        }
    }

    private void handle(Socket socket) {
        try {
            socket.setSoTimeout(30_000);
            InputStream in = socket.getInputStream();
            String requestLine = readLine(in);
            String range = null;
            String line;
            while ((line = readLine(in)) != null && line.length() > 0) {
                int colon = line.indexOf(':');
                if (colon > 0 && line.substring(0, colon).trim().equalsIgnoreCase("Range")) range = line.substring(colon + 1).trim();
            }
            OutputStream out = new BufferedOutputStream(socket.getOutputStream(), 64 * 1024);
            respond(requestLine, range, out);
            out.flush();
        } catch (IOException ignored) {
            // the phone went away; it resumes with Range next time
        } finally {
            try {
                socket.close();
            } catch (IOException ignored) {
                // closing
            }
        }
    }

    private void respond(String requestLine, String range, OutputStream out) throws IOException {
        String[] parts = requestLine == null ? new String[0] : requestLine.split(" ");
        if (parts.length < 2 || !parts[0].equals("GET")) {
            status(out, 405, "Method Not Allowed");
            return;
        }
        String target = parts[1];
        int q = target.indexOf('?');
        String path = q < 0 ? target : target.substring(0, q);
        String query = q < 0 ? "" : target.substring(q + 1);
        if (!MessageDigest.isEqual(token, param(query, "token").getBytes())) {
            status(out, 403, "Forbidden");
            return;
        }
        File file = path.startsWith("/media/") ? index.file(path.substring("/media/".length())) : null;
        if (file == null || !file.isFile()) {
            status(out, 404, "Not Found");
            return;
        }
        long length = file.length();
        long from = 0;
        if (range != null && range.startsWith("bytes=")) {
            String spec = range.substring(6);
            int dash = spec.indexOf('-');
            try {
                from = Long.parseLong(spec.substring(0, dash));
            } catch (RuntimeException e) {
                from = 0;
            }
            if (from < 0 || from >= length) {
                status(out, 416, "Range Not Satisfiable");
                return;
            }
        }
        String name = file.getName().toLowerCase(java.util.Locale.ROOT);
        String type = name.endsWith(".mp4") ? "video/mp4" : name.endsWith(".3gp") ? "video/3gpp"
                : name.endsWith(".png") ? "image/png" : "image/jpeg";
        StringBuilder h = new StringBuilder();
        h.append(from > 0 ? "HTTP/1.1 206 Partial Content\r\n" : "HTTP/1.1 200 OK\r\n");
        h.append("Content-Type: ").append(type).append("\r\n");
        h.append("Content-Length: ").append(length - from).append("\r\n");
        if (from > 0) h.append("Content-Range: bytes ").append(from).append('-').append(length - 1).append('/').append(length).append("\r\n");
        h.append("Accept-Ranges: bytes\r\n");
        h.append("X-Content-SHA256: ").append(sha256(file)).append("\r\n");
        h.append("Connection: close\r\n\r\n");
        out.write(h.toString().getBytes("US-ASCII"));
        RandomAccessFile raf = new RandomAccessFile(file, "r");
        try {
            raf.seek(from);
            byte[] buf = new byte[64 * 1024];
            int n;
            while ((n = raf.read(buf)) > 0) out.write(buf, 0, n);
        } finally {
            raf.close();
        }
    }

    private synchronized String sha256(File file) throws IOException {
        String key = file.getAbsolutePath() + "|" + file.length() + "|" + file.lastModified();
        String cached = hashes.get(key);
        if (cached != null) return cached;
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-256");
            InputStream in = new FileInputStream(file);
            try {
                byte[] buf = new byte[64 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) d.update(buf, 0, n);
            } finally {
                in.close();
            }
            StringBuilder sb = new StringBuilder();
            for (byte b : d.digest()) sb.append(String.format("%02x", b));
            hashes.put(key, sb.toString());
            return sb.toString();
        } catch (java.security.NoSuchAlgorithmException e) {
            throw new IOException(e);
        }
    }

    private static void status(OutputStream out, int code, String text) throws IOException {
        out.write(("HTTP/1.1 " + code + " " + text + "\r\nContent-Length: 0\r\nConnection: close\r\n\r\n").getBytes("US-ASCII"));
    }

    private static String param(String query, String name) {
        for (String pair : query.split("&")) {
            int eq = pair.indexOf('=');
            if (eq > 0 && pair.substring(0, eq).equals(name)) return pair.substring(eq + 1);
        }
        return "";
    }

    /** One CRLF-terminated header line (ASCII), or null at end of stream; lines over 8 KiB are refused. */
    private static String readLine(InputStream in) throws IOException {
        StringBuilder sb = new StringBuilder();
        int c;
        while ((c = in.read()) >= 0) {
            if (c == '\n') break;
            if (c != '\r') sb.append((char) c);
            if (sb.length() > 8192) throw new IOException("header line too long");
        }
        return c < 0 && sb.length() == 0 ? null : sb.toString();
    }
}
