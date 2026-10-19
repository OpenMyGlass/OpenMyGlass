package org.openmyglass.glass.nav;

import java.util.HashSet;
import java.util.LinkedHashMap;
import java.util.Map;
import java.util.Set;

/** Maneuver icons by SHA-256 (hex) → PNG bytes; least-recently-used eviction; remembers pending requests. */
public final class IconCache {
    private final Map<String, byte[]> icons;
    private final Set<String> requested = new HashSet<String>();

    public IconCache(final int maxEntries) {
        icons = new LinkedHashMap<String, byte[]>(16, 0.75f, true) {
            @Override protected boolean removeEldestEntry(Map.Entry<String, byte[]> eldest) {
                return size() > maxEntries;
            }
        };
    }

    public synchronized byte[] get(String hex) {
        return icons.get(hex);
    }

    public synchronized void put(String hex, byte[] png) {
        icons.put(hex, png);
        requested.remove(hex);
    }

    /** Forgets pending requests: a new session asks again for icons whose request was lost with the old one. */
    public synchronized void clearRequested() {
        requested.clear();
    }

    /** True if the icon is neither cached nor already requested: the caller should request it now. */
    public synchronized boolean markRequested(String hex) {
        return !icons.containsKey(hex) && requested.add(hex);
    }
}
