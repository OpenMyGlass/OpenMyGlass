import SwiftUI
import UIKit

/// What the screen shows. Every core call runs on `CompanionProcess.coreCalls`; the results are
/// published here, on the main actor.
@MainActor
final class CompanionModel: ObservableObject {
    @Published var host = "127.0.0.1"
    @Published var port = "7700"
    @Published var allowPairing = false
    @Published private(set) var snapshot: CoreSnapshot?
    @Published private(set) var noteCount = 0
    @Published private(set) var recentEvents: [String] = []
    @Published private(set) var problem: String?
    @Published private(set) var wantsLink = false

    private var process: CompanionProcess?

    init() {
        let deviceName = UIDevice.current.name
        DispatchQueue.global().async { [weak self] in
            do {
                let process = try CompanionProcess.shared(
                    deviceName: deviceName,
                    onEvents: { events in
                        DispatchQueue.main.async {
                            MainActor.assumeIsolated { self?.received(events) }
                        }
                    },
                    onOpenFailed: { why in
                        DispatchQueue.main.async {
                            MainActor.assumeIsolated { self?.problem = "Could not open the link: \(why)" }
                        }
                    }
                )
                DispatchQueue.main.async {
                    MainActor.assumeIsolated {
                        self?.process = process
                        self?.refresh()
                    }
                }
            } catch {
                DispatchQueue.main.async {
                    MainActor.assumeIsolated { self?.problem = "The core did not start: \(error)" }
                }
            }
        }
    }

    func connect() {
        guard let process else { return }
        guard let portNumber = UInt16(port) else {
            problem = "The port must be a number from 0 to 65535."
            return
        }
        let host = host.trimmingCharacters(in: .whitespaces)
        let allowPairing = allowPairing
        problem = nil
        wantsLink = true
        process.link.connect(allowPairing: allowPairing) { cancelled in
            try TcpLink(host: host, port: portNumber, cancelled: cancelled)
        }
    }

    func disconnect() {
        wantsLink = false
        process?.link.disconnect()
        refresh()
    }

    private func received(_ events: [CoreEvent]) {
        if events.contains(where: { if case .linkRunning = $0 { return true } else { return false } }) {
            problem = nil
        }
        recentEvents = Array((events.reversed().map { "\($0)" } + recentEvents).prefix(8))
        refresh()
    }

    private func refresh() {
        guard let process else { return }
        process.coreCalls.async { [weak self] in
            let snapshot = process.core.snapshot()
            let count = process.core.notes().count
            Task { @MainActor in
                self?.snapshot = snapshot
                self?.noteCount = count
            }
        }
    }
}

struct ContentView: View {
    @StateObject private var model = CompanionModel()

    var body: some View {
        Form {
            Section("Glass") {
                TextField("Host", text: $model.host)
                    .textInputAutocapitalization(.never)
                    .autocorrectionDisabled()
                TextField("Port", text: $model.port)
                    .keyboardType(.numberPad)
                Toggle("Allow pairing", isOn: $model.allowPairing)
                Button("Connect") { model.connect() }
                    .buttonStyle(.borderless)
                Button("Disconnect") { model.disconnect() }
                    .buttonStyle(.borderless)
            }
            Section("Status") {
                ForEach(statusLines, id: \.self) { Text($0) }
                if let problem = model.problem { Text(problem) }
            }
            if !model.recentEvents.isEmpty {
                Section("Events") {
                    ForEach(Array(model.recentEvents.enumerated()), id: \.offset) { Text($0.element) }
                }
            }
        }
    }

    private var statusLines: [String] {
        guard let snapshot = model.snapshot else { return ["Starting"] }
        var lines = ["Link: \(phaseText(snapshot.phase))"]
        if let name = snapshot.glass.name { lines.append("Glass: \(name)") }
        if let battery = snapshot.glass.batteryPercent {
            lines.append("Battery: \(battery) %" + (snapshot.glass.charging == true ? ", charging" : ""))
        }
        if let version = snapshot.glass.softwareVersion { lines.append("Software: \(version)") }
        lines.append("Notes: \(model.noteCount)")
        return lines
    }

    private func phaseText(_ phase: LinkPhase) -> String {
        switch phase {
        case .down: return model.wantsLink ? "connecting" : "down"
        case .handshake: return "handshake"
        case .running: return "connected"
        }
    }
}
