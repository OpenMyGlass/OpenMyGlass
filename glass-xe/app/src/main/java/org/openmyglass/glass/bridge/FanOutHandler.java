package org.openmyglass.glass.bridge;

import java.util.ArrayList;
import java.util.List;

import org.openmyglass.protocol.v1.Bridge;

/** One bridge session feeding several features (navigation card, notifications); each ignores frames it does not use. */
public final class FanOutHandler implements BridgeServer.FrameHandler {
    private final List<BridgeServer.FrameHandler> handlers;

    public FanOutHandler(List<BridgeServer.FrameHandler> handlers) {
        this.handlers = new ArrayList<BridgeServer.FrameHandler>(handlers);
    }

    @Override
    public void onSessionStarted(BridgeServer.Sender sender) {
        for (BridgeServer.FrameHandler h : handlers) h.onSessionStarted(sender);
    }

    @Override
    public void onFrame(Bridge.Frame frame) {
        for (BridgeServer.FrameHandler h : handlers) h.onFrame(frame);
    }

    @Override
    public void onSessionEnded(BridgeServer.Sender sender) {
        for (BridgeServer.FrameHandler h : handlers) h.onSessionEnded(sender);
    }
}
