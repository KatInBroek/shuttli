import SwiftUI
import UIKit
import UniformTypeIdentifiers

@main
struct ShuttliApp: App {
    @Environment(\.scenePhase) private var scenePhase
    @StateObject private var state = AppState()

    var body: some Scene {
        WindowGroup {
            RootView()
                .environmentObject(state)
        }
        .onChange(of: scenePhase) { phase in
            switch phase {
            case .active:
                state.enterForeground()
            case .background:
                state.enterBackground()
            case .inactive:
                break
            @unknown default:
                state.enterBackground()
            }
        }
    }
}

@MainActor
final class AppState: ObservableObject {
    @Published private(set) var draft: String?
    @Published private(set) var historyCount: UInt32 = 0
    @Published private(set) var historyRows: [MobileHistoryRow] = []
    @Published private(set) var deviceRows: [MobileDeviceRow] = []
    @Published private(set) var copyStatusKey: String?
    @Published private(set) var connectedPeerCount: UInt32 = 0
    @Published private(set) var connectionStatusKey = "waiting_for_devices"
    private let session = MobileSession()
    private var refreshTask: Task<Void, Never>?

    func enterForeground() {
        refreshTask?.cancel()
        _ = session.enterForeground()
        guard let identity = DeviceIdentityStore.loadOrCreate() else {
            connectionStatusKey = "identity_unavailable"
            return
        }
        guard let address = TailnetAddress.currentIPv4() else {
            connectionStatusKey = "tailscale_unavailable"
            return
        }
        let error = session.startListener(
            identityBytes: identity,
            tailscaleIp: address,
            name: UIDevice.current.name
        )
        connectionStatusKey = error.isEmpty ? "waiting_for_devices" : "listener_unavailable"
        updateSnapshot()
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.updateSnapshot()
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    func enterBackground() {
        refreshTask?.cancel()
        refreshTask = nil
        session.enterBackground()
        draft = nil
        connectedPeerCount = 0
    }

    private func updateSnapshot() {
        historyRows = session.historyRows()
        historyCount = UInt32(historyRows.count)
        deviceRows = session.deviceRows()
        connectedPeerCount = session.connectedPeersCount()
        if connectedPeerCount > 0 { connectionStatusKey = "devices_connected" }
    }

    func body(for row: MobileHistoryRow) -> Data {
        session.historyBody(eventKey: row.eventKey)
    }

    func copyToPhone(_ row: MobileHistoryRow) {
        let data = body(for: row)
        guard !data.isEmpty else {
            copyStatusKey = "content_unavailable"
            return
        }
        switch row.kind {
        case .text:
            guard let value = String(data: data, encoding: .utf8) else {
                copyStatusKey = "content_unavailable"
                return
            }
            UIPasteboard.general.string = value
            copyStatusKey = UIPasteboard.general.string == value ? "copied_to_phone" : "copy_uncertain"
        case .image:
            UIPasteboard.general.setData(data, forPasteboardType: UTType.png.identifier)
            copyStatusKey = UIPasteboard.general.data(forPasteboardType: UTType.png.identifier) == data
                ? "copied_to_phone" : "copy_uncertain"
        }
    }

    func setReceive(_ enabled: Bool, for row: MobileDeviceRow) {
        _ = session.setReceive(peerId: row.id, enabled: enabled)
        updateSnapshot()
    }

    func clearHistory() {
        session.clearHistory()
        copyStatusKey = nil
        updateSnapshot()
    }

    func importText(_ text: String) {
        guard !text.isEmpty, text.utf8.count <= 1_048_576 else { return }
        draft = text
    }
}
