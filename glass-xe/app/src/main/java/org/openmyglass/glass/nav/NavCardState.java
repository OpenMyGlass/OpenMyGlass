package org.openmyglass.glass.nav;

import java.io.IOException;

import org.openmyglass.glass.bridge.BridgeServer;
import org.openmyglass.protocol.v1.Bridge;

/**
 * The navigation card shown on Glass, driven by bridge frames. Unknown icons are requested once; if the
 * phone disappears without NavEnded, the card is kept for a grace period (the phone usually reconnects and
 * resends its state) and then ended. Frames arrive on the bridge session thread.
 */
public final class NavCardState implements BridgeServer.FrameHandler {

    public interface Listener {
        void onChanged(Card card);
        void onEnded();
    }

    public interface Cancellable {
        void cancel();
    }

    /** Runs a task later; injected so tests control time. */
    public interface Scheduler {
        Cancellable schedule(Runnable task, long delayMs);
    }

    /** What the renderers draw. Absent values are null. */
    public static final class Card {
        public final Bridge.Maneuver maneuver;
        public final Integer distanceM;
        public final String instruction;
        public final String street;
        public final String eta;
        public final String source;
        public final String iconHex;
        public final byte[] iconPng;

        Card(Bridge.NavState nav, String iconHex, byte[] iconPng) {
            this.maneuver = nav.getManeuver();
            this.distanceM = nav.hasDistanceM() ? nav.getDistanceM() : null;
            this.instruction = nav.hasInstruction() ? nav.getInstruction() : null;
            this.street = nav.hasStreet() ? nav.getStreet() : null;
            this.eta = nav.hasEta() ? nav.getEta() : null;
            this.source = nav.getSource();
            this.iconHex = iconHex;
            this.iconPng = iconPng;
        }
    }

    private final IconCache icons;
    private final Listener listener;
    private final Scheduler scheduler;
    private final long graceMs;
    private BridgeServer.Sender sender;
    private Bridge.NavState nav;
    private Cancellable graceTimer;

    public NavCardState(IconCache icons, Listener listener, Scheduler scheduler, long graceMs) {
        this.icons = icons;
        this.listener = listener;
        this.scheduler = scheduler;
        this.graceMs = graceMs;
    }

    @Override
    public synchronized void onSessionStarted(BridgeServer.Sender sender) {
        this.sender = sender;
        cancelGrace();
    }

    @Override
    public synchronized void onFrame(Bridge.Frame frame) {
        if (frame.hasNav()) {
            nav = frame.getNav();
            publish();
        } else if (frame.hasNavEnded()) {
            end();
        } else if (frame.hasIcon()) {
            String hex = hex(frame.getIcon().getSha256().toByteArray());
            icons.put(hex, frame.getIcon().getPng().toByteArray());
            if (nav != null && nav.hasIconSha256() && hex.equals(hex(nav.getIconSha256().toByteArray()))) publish();
        }
    }

    @Override
    public synchronized void onSessionEnded() {
        sender = null;
        if (nav == null) return;
        cancelGrace();
        graceTimer = scheduler.schedule(new Runnable() {
            @Override public void run() {
                synchronized (NavCardState.this) {
                    graceTimer = null;
                    if (sender == null && nav != null) end();
                }
            }
        }, graceMs);
    }

    private void publish() {
        String hex = nav.hasIconSha256() ? hex(nav.getIconSha256().toByteArray()) : null;
        byte[] png = hex == null ? null : icons.get(hex);
        if (hex != null && png == null && sender != null && icons.markRequested(hex)) {
            try {
                sender.send(Bridge.Frame.newBuilder().setIconRequest(
                        Bridge.IconRequest.newBuilder().setSha256(nav.getIconSha256())).build());
            } catch (IOException e) {
                // the session is ending; the phone resends state and icons after reconnecting
            }
        }
        listener.onChanged(new Card(nav, hex, png));
    }

    private void end() {
        nav = null;
        cancelGrace();
        listener.onEnded();
    }

    private void cancelGrace() {
        if (graceTimer != null) {
            graceTimer.cancel();
            graceTimer = null;
        }
    }

    private static String hex(byte[] bytes) {
        StringBuilder sb = new StringBuilder(bytes.length * 2);
        for (byte b : bytes) sb.append(String.format("%02x", b));
        return sb.toString();
    }
}
