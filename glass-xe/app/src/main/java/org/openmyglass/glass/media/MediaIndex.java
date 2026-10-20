package org.openmyglass.glass.media;

import java.io.File;
import java.security.MessageDigest;
import java.security.NoSuchAlgorithmException;
import java.util.ArrayList;
import java.util.Collections;
import java.util.Comparator;
import java.util.HashMap;
import java.util.List;
import java.util.Locale;
import java.util.Map;

/**
 * Photos and videos in the camera folder. Ids are stable for the same file (path, size, modification time), so
 * the phone remembers what it already synced; only ids from the latest scan resolve to files (no paths from
 * the network). Files modified within {@code settleMs} are left out: a video being recorded is still growing.
 * SHA-256 hashes are kept per file version, so a long video is hashed once, not on every pull.
 */
public final class MediaIndex {

    public static final class Item {
        public final String id;
        public final String name;
        public final boolean video;
        public final long size;
        public final long takenAtMs;
        final File file;

        Item(String id, File file, boolean video) {
            this.id = id;
            this.name = file.getName();
            this.video = video;
            this.size = file.length();
            this.takenAtMs = file.lastModified();
            this.file = file;
        }
    }

    public static final long DEFAULT_SETTLE_MS = 15_000;

    private final File dir;
    private final long settleMs;
    private final Map<String, File> byId = new HashMap<String, File>();
    private final Map<String, String> hashes = new HashMap<String, String>();
    /** For tests: how many files were hashed. */
    int hashComputations;

    public MediaIndex(File dir) {
        this(dir, DEFAULT_SETTLE_MS);
    }

    public MediaIndex(File dir, long settleMs) {
        this.dir = dir;
        this.settleMs = settleMs;
    }

    public long settleMs() {
        return settleMs;
    }

    /** SHA-256 (hex) of the file's current version; computed once per path, size and modification time. */
    public String sha256(File file) throws java.io.IOException {
        String key = file.getAbsolutePath() + "|" + file.length() + "|" + file.lastModified();
        synchronized (hashes) {
            String cached = hashes.get(key);
            if (cached != null) return cached;
        }
        try {
            MessageDigest d = MessageDigest.getInstance("SHA-256");
            java.io.InputStream in = new java.io.FileInputStream(file);
            try {
                byte[] buf = new byte[64 * 1024];
                int n;
                while ((n = in.read(buf)) > 0) d.update(buf, 0, n);
            } finally {
                in.close();
            }
            StringBuilder sb = new StringBuilder();
            for (byte b : d.digest()) sb.append(String.format("%02x", b));
            synchronized (hashes) {
                hashComputations++;
                hashes.put(key, sb.toString());
            }
            return sb.toString();
        } catch (NoSuchAlgorithmException e) {
            throw new java.io.IOException(e);
        }
    }

    public File directory() {
        return dir;
    }

    /** The media now in the folder, oldest first. */
    public synchronized List<Item> scan() {
        List<Item> items = new ArrayList<Item>();
        File[] files = dir.listFiles();
        byId.clear();
        if (files == null) return items;
        long now = System.currentTimeMillis();
        for (File f : files) {
            if (!f.isFile()) continue;
            if (now - f.lastModified() < settleMs) continue;   // still being written
            String name = f.getName().toLowerCase(Locale.ROOT);
            boolean photo = name.endsWith(".jpg") || name.endsWith(".jpeg") || name.endsWith(".png");
            boolean video = name.endsWith(".mp4") || name.endsWith(".3gp");
            if (!photo && !video) continue;
            Item item = new Item(id(f), f, video);
            items.add(item);
            byId.put(item.id, f);
        }
        Collections.sort(items, new Comparator<Item>() {
            @Override public int compare(Item a, Item b) {
                return a.takenAtMs < b.takenAtMs ? -1 : a.takenAtMs > b.takenAtMs ? 1 : a.name.compareTo(b.name);
            }
        });
        return items;
    }

    /** The file for an id only if it is still that exact version (same path, size and modification time). */
    public synchronized File fileIfUnchanged(String id) {
        File f = file(id);
        return f != null && f.isFile() && id(f).equals(id) ? f : null;
    }

    /** The file for an id from the latest scan, or null. */
    public synchronized File file(String id) {
        if (byId.isEmpty()) scan();
        return byId.get(id);
    }

    private static String id(File f) {
        try {
            MessageDigest sha = MessageDigest.getInstance("SHA-1");
            byte[] d = sha.digest((f.getAbsolutePath() + "|" + f.length() + "|" + f.lastModified()).getBytes("UTF-8"));
            StringBuilder sb = new StringBuilder();
            for (int i = 0; i < 10; i++) sb.append(String.format("%02x", d[i]));
            return sb.toString();
        } catch (NoSuchAlgorithmException | java.io.UnsupportedEncodingException e) {
            throw new IllegalStateException(e);
        }
    }
}
