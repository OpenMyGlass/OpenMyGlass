import Foundation

/// Opens a link and blocks until it is up. Throws if it fails. It should give up, the same way,
/// once `cancelled` returns true: nobody wants this link any more.
typealias LinkOpener = (_ cancelled: () -> Bool) throws -> ByteLink

/// Keeps a link to the Glass while one is wanted, reconnecting after a loss. The threads are the
/// ones `docs/companion-mobile.md` asks for: a reader and a pump per link, plus one that
/// supervises.
///
/// `onEvents` gets the events of a link, and also an empty list each time an attempt ends, because
/// a link that simply dropped has no event of its own and the screen must still stop saying it is
/// connected. `onOpenFailed` says why a wanted link did not open. Both run on the link's threads.
final class LinkController: @unchecked Sendable {
    private let core: CompanionCoreProtocol
    private let freeBytes: () -> UInt64?
    private let backoff: ReconnectBackoffProtocol
    private let onOpenFailed: (String) -> Void
    private let onEvents: ([CoreEvent]) -> Void

    /// Ends the wait between pumps.
    private let wake = DispatchSemaphore(value: 0)
    /// Ends the wait between attempts. Only the owner asking for something does that.
    private let retry = DispatchSemaphore(value: 0)

    private final class Request {
        let opener: LinkOpener
        let allowPairing: Bool
        init(opener: @escaping LinkOpener, allowPairing: Bool) {
            self.opener = opener
            self.allowPairing = allowPairing
        }
    }

    private let lock = NSLock()
    private var open: Request?
    private var current: ByteLink?
    private var supervising = false

    init(
        core: CompanionCoreProtocol,
        freeBytes: @escaping () -> UInt64?,
        backoff: ReconnectBackoffProtocol = ReconnectBackoff(),
        onOpenFailed: @escaping (String) -> Void,
        onEvents: @escaping ([CoreEvent]) -> Void
    ) {
        self.core = core
        self.freeBytes = freeBytes
        self.backoff = backoff
        self.onOpenFailed = onOpenFailed
        self.onEvents = onEvents
    }

    /// Keep a link open with what `opener` returns, instead of any link there is now.
    func connect(allowPairing: Bool, opener: @escaping LinkOpener) {
        lock.lock()
        open = Request(opener: opener, allowPairing: allowPairing)
        let link = current
        let start = !supervising
        supervising = true
        lock.unlock()
        link?.close()
        if start {
            Thread.detachNewThread { [self] in supervise() }
        }
        retry.signal()
    }

    /// Close the link and stay down.
    func disconnect() {
        lock.lock()
        open = nil
        let link = current
        lock.unlock()
        link?.close()
        retry.signal()
    }

    private func wanted() -> Request? {
        lock.lock()
        defer { lock.unlock() }
        return open
    }

    private func wants(_ request: Request) -> Bool {
        wanted() === request
    }

    private func supervise() {
        while true {
            // Under the lock, so that a connect() either sees this thread gone or is seen here.
            lock.lock()
            guard let request = open else {
                supervising = false
                lock.unlock()
                return
            }
            lock.unlock()

            var lastedMs: UInt64 = 0
            if let link = openLink(request) {
                let started = DispatchTime.now()
                // Before the check: connect() sets `open` and then closes `current`, so a link
                // nobody wants any more is either not run here or closed there.
                lock.lock()
                current = link
                lock.unlock()
                if wants(request) { runLink(link, request) } else { link.close() }
                lock.lock()
                current = nil
                lock.unlock()
                lastedMs = (DispatchTime.now().uptimeNanoseconds - started.uptimeNanoseconds) / 1_000_000
            }
            onEvents(core.pump().events)
            // Emptied before the check: a request after the check ends the wait.
            while retry.wait(timeout: .now()) == .success {}
            if wants(request) {
                let waitMs = Int(backoff.nextWaitMs(lastedMs: lastedMs))
                _ = retry.wait(timeout: .now() + .milliseconds(waitMs))
            }
            if !wants(request) { backoff.reset() }
        }
    }

    private func openLink(_ request: Request) -> ByteLink? {
        do {
            return try request.opener { !wants(request) }
        } catch {
            if wants(request) { onOpenFailed("\(error)") }
            return nil
        }
    }

    private func runLink(_ link: ByteLink, _ request: Request) {
        core.setFreeBytes(freeBytes: freeBytes())
        core.linkUp(profile: link.profile, allowPairing: request.allowPairing)
        let ended = Atomic()
        let readerDone = DispatchSemaphore(value: 0)
        let core = self.core
        let wake = self.wake
        Thread.detachNewThread {
            var buffer = [UInt8](repeating: 0, count: Self.readBuffer)
            while let n = try? link.read(into: &buffer), n > 0 {
                core.onBytes(data: Data(buffer[0..<n]))
                wake.signal()
            }
            // An error is the same as the end of the stream: the link is gone.
            ended.set()
            wake.signal()
            readerDone.signal()
        }

        while !ended.isSet {
            _ = wake.wait(timeout: .now() + .milliseconds(Self.pumpIntervalMs))
            while wake.wait(timeout: .now()) == .success {}
            // Read before the pump, so what was queued before the owner stopped, or asked for
            // another link, still goes.
            let stopping = !wants(request)
            let out = core.pump()
            var failed = false
            if !out.outgoing.isEmpty {
                do { try link.write(out.outgoing) } catch { failed = true }
            }
            if !out.events.isEmpty { onEvents(out.events) }
            if failed || out.closeLink || stopping { break }
        }
        link.close()
        readerDone.wait()
        core.linkDown()
    }

    private final class Atomic: @unchecked Sendable {
        private let lock = NSLock()
        private var value = false
        var isSet: Bool {
            lock.lock()
            defer { lock.unlock() }
            return value
        }
        func set() {
            lock.lock()
            value = true
            lock.unlock()
        }
    }

    private static let pumpIntervalMs = 250
    private static let readBuffer = 64 * 1024
}
