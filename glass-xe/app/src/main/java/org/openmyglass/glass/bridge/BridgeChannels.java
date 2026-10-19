package org.openmyglass.glass.bridge;

import java.util.ArrayList;
import java.util.List;

/** Which channels the bridge listens on: RFCOMM always; the unauthenticated TCP port only on debuggable builds. */
public final class BridgeChannels {
    private BridgeChannels() {}

    public static List<ServerChannel> forBuild(ServerChannel rfcomm, int tcpPort, boolean debuggable) {
        List<ServerChannel> channels = new ArrayList<ServerChannel>();
        channels.add(rfcomm);
        if (debuggable) channels.add(new TcpServerChannel(tcpPort));
        return channels;
    }
}
