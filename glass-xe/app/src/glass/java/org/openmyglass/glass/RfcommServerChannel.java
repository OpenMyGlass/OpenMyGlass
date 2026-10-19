package org.openmyglass.glass;

import android.bluetooth.BluetoothAdapter;
import android.bluetooth.BluetoothServerSocket;
import android.bluetooth.BluetoothSocket;

import java.io.IOException;
import java.io.InputStream;
import java.io.OutputStream;
import java.util.UUID;

import org.openmyglass.glass.bridge.ServerChannel;

/** Secure RFCOMM listener (bonded phones only) for the bridge; the phone connects with the same UUID. */
final class RfcommServerChannel implements ServerChannel {
    static final UUID BRIDGE_UUID = UUID.fromString("8a0dcb95-c455-453f-87b5-d69ddc82b743");
    static final String NAME = "OpenMyGlass Bridge";

    private volatile BluetoothServerSocket server;

    // Glass runs API 19: BLUETOOTH (in the manifest) is the permission; BLUETOOTH_CONNECT is API 31+
    @android.annotation.SuppressLint("MissingPermission")
    @Override
    public void open() throws IOException {
        BluetoothAdapter adapter = BluetoothAdapter.getDefaultAdapter();
        if (adapter == null || !adapter.isEnabled()) throw new IOException("Bluetooth off");
        server = adapter.listenUsingRfcommWithServiceRecord(NAME, BRIDGE_UUID);
    }

    @Override
    public Connection accept() throws IOException {
        BluetoothServerSocket s = server;
        if (s == null) throw new IOException("not listening");
        final BluetoothSocket socket = s.accept();
        return new Connection() {
            @Override public InputStream input() throws IOException { return socket.getInputStream(); }
            @Override public OutputStream output() throws IOException { return socket.getOutputStream(); }
            @Override public void close() throws IOException { socket.close(); }
        };
    }

    @Override
    public void close() {
        BluetoothServerSocket s = server;
        server = null;
        if (s != null) {
            try {
                s.close();
            } catch (IOException ignored) {
                // closing
            }
        }
    }
}
