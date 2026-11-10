import Foundation

/// What must outlive any one screen, one per process: the core and the link. A second core on
/// the same directory is refused while the first is alive, so there is only ever this one.
final class CompanionProcess: @unchecked Sendable {
    private static var instance: CompanionProcess?
    private static let guardLock = NSLock()

    /// The one for this process, made on the first call. Throws `CoreFailure` if the core cannot
    /// open its state, a full disk for instance, and the next call tries again. Opening reads
    /// the disk, so call it off the main thread.
    ///
    /// The handlers are fixed when the process is made, before any thread starts, and run on
    /// background threads: `onEvents` gets the events of the link, and of the poller when no link
    /// is up. Later calls get the process as the first call made it.
    static func shared(
        deviceName: String,
        onEvents: @escaping ([CoreEvent]) -> Void,
        onOpenFailed: @escaping (String) -> Void
    ) throws -> CompanionProcess {
        guardLock.lock()
        defer { guardLock.unlock() }
        if let instance { return instance }
        let made = try CompanionProcess(deviceName: deviceName, onEvents: onEvents, onOpenFailed: onOpenFailed)
        instance = made
        return made
    }

    let core: CompanionCore
    private(set) var link: LinkController!

    /// The one queue the screen calls the core on. A call waits for the core's lock, which can be
    /// held for seconds while a big attachment is hashed, so none may run on the main thread.
    let coreCalls = DispatchQueue(label: "org.openmyglass.core-calls")

    private init(
        deviceName: String,
        onEvents: @escaping ([CoreEvent]) -> Void,
        onOpenFailed: @escaping (String) -> Void
    ) throws {
        let support = try FileManager.default.url(
            for: .applicationSupportDirectory, in: .userDomainMask, appropriateFor: nil, create: true
        )
        let dataDir = support.appendingPathComponent("omg", isDirectory: true)
        try FileManager.default.createDirectory(at: dataDir, withIntermediateDirectories: true)
        let version = Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "0"
        core = try CompanionCore(
            dataDir: dataDir.path,
            deviceName: deviceName,
            appVersion: version,
            stt: UnavailableSpeechToText(),
            tts: UnavailableTextToSpeech()
        )
        link = LinkController(
            core: core,
            freeBytes: {
                let values = try? dataDir.resourceValues(forKeys: [.volumeAvailableCapacityForImportantUsageKey])
                return values?.volumeAvailableCapacityForImportantUsage.map { UInt64(max($0, 0)) }
            },
            onOpenFailed: onOpenFailed,
            onEvents: onEvents
        )
        // A note can change with no link, when speech finishes, and nothing pumps then.
        let core = self.core
        Thread.detachNewThread {
            while true {
                Thread.sleep(forTimeInterval: 0.5)
                let events = core.poll()
                if !events.isEmpty { onEvents(events) }
            }
        }
    }
}
