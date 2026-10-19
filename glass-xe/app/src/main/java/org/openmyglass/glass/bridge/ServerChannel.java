package org.openmyglass.glass.bridge;

import java.io.Closeable;
import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;

/** Where the bridge accepts phone connections: TCP (debug, Wi-Fi) or Bluetooth RFCOMM. */
public interface ServerChannel extends Closeable {

    interface Connection extends Closeable {
        InputStream input() throws IOException;
        OutputStream output() throws IOException;
    }

    /** Starts listening; called again to reopen after {@link #accept()} failed (e.g. Bluetooth turned off). */
    void open() throws IOException;

    /** Blocks until a phone connects. */
    Connection accept() throws IOException;

    @Override
    void close();
}
