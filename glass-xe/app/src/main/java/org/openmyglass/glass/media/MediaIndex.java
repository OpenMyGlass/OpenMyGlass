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
 * the network).
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

    private final File dir;
    private final Map<String, File> byId = new HashMap<String, File>();

    public MediaIndex(File dir) {
        this.dir = dir;
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
        for (File f : files) {
            if (!f.isFile()) continue;
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
