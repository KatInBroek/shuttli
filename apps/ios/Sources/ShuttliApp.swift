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
    @Published private(set) var imageDraft: Data?
    @Published private(set) var historyCount: UInt32 = 0
    @Published private(set) var historyRows: [MobileHistoryRow] = []
    @Published private(set) var deviceRows: [MobileDeviceRow] = []
    @Published private(set) var transferRows: [MobileTransferRow] = []
    @Published private(set) var sendStatusKey: String?
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
        for (id, directions) in DevicePolicyStore.load() {
            _ = session.restoreDeviceDirections(peerId: id, send: directions.send, receive: directions.receive)
        }
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
        imageDraft = nil
        connectedPeerCount = 0
    }

    private func updateSnapshot() {
        historyRows = session.historyRows()
        historyCount = UInt32(historyRows.count)
        deviceRows = session.deviceRows()
        transferRows = session.transferRows()
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
        if DevicePolicyStore.save(id: row.id, send: row.send, receive: enabled) {
            _ = session.setReceive(peerId: row.id, enabled: enabled)
        }
        updateSnapshot()
    }

    func setSend(_ enabled: Bool, for row: MobileDeviceRow) {
        if DevicePolicyStore.save(id: row.id, send: enabled, receive: row.receive) {
            _ = session.setSend(peerId: row.id, enabled: enabled)
        }
        updateSnapshot()
    }

    var allowedSendCount: Int {
        deviceRows.filter { $0.online && $0.send }.count
    }

    func sendDraft() {
        let queued: UInt32
        if let imageDraft {
            queued = session.sendImage(pngBytes: imageDraft)
        } else if let draft {
            queued = session.sendText(text: draft)
        } else { return }
        sendStatusKey = queued > 0 ? "send_queued" : "no_send_targets"
        if queued > 0 { self.draft = nil; imageDraft = nil }
        updateSnapshot()
    }

    func resend(_ row: MobileHistoryRow) {
        let queued = session.resendLocal(key: row.eventKey)
        sendStatusKey = queued > 0 ? "send_queued" : "no_send_targets"
        updateSnapshot()
    }

    func clearHistory() {
        session.clearHistory()
        draft = nil
        imageDraft = nil
        copyStatusKey = nil
        enterBackground()
        enterForeground()
        updateSnapshot()
    }

    func importText(_ text: String) {
        guard !text.isEmpty, text.utf8.count <= 1_048_576 else { return }
        draft = text
        imageDraft = nil
    }

    func importImage(_ image: UIImage) {
        let pixels = image.size.width * image.scale * image.size.height * image.scale
        guard pixels > 0, pixels <= 4_194_304,
              image.size.width * image.scale <= 16_384,
              image.size.height * image.scale <= 16_384,
              let data = image.pngData(), data.count <= 8 * 1024 * 1024 else {
            sendStatusKey = "image_too_large"
            return
        }
        imageDraft = data
        draft = nil
    }
}
