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
    @Published private(set) var fingerprint = ""
    @Published private(set) var draft: String?
    @Published private(set) var imageDraft: Data?
    @Published private(set) var historyCount: UInt32 = 0
    @Published private(set) var historyRows: [MobileHistoryRow] = []
    @Published private(set) var deviceRows: [MobileDeviceRow] = []
    @Published private(set) var transferRows: [MobileTransferRow] = []
    @Published private(set) var sendStatusKey: String?
    @Published private(set) var copyStatusKey: String?
    @Published private(set) var settingsStatusKey: String?
    @Published private(set) var connectedPeerCount: UInt32 = 0
    @Published private(set) var connectionStatusKey = "waiting_for_devices"
    @Published private(set) var historyModeKey = "content"
    @Published private(set) var historyLimit = 20
    private let session = MobileSession()
    private var refreshTask: Task<Void, Never>?
    private var sendEvents: Set<String> = []
    private var active = false
    private var identityBytes: Data?
    private var listenerAddress: String?
    private var permissionsRestored = false

    static func sendStatus(_ states: [MobileTransferState]) -> String {
        if states.isEmpty { return "transfer_unknown" }
        if states.contains(.queued) || states.contains(.sending) { return "send_queued" }
        if states.allSatisfy({ $0 == .applied }) { return "transfer_applied" }
        if states.contains(.applied) { return "send_partial" }
        if states.contains(.unknown) { return "transfer_unknown" }
        return "transfer_failed"
    }

    func enterForeground() {
        guard !active else { return }
        active = true
        refreshTask?.cancel()
        _ = session.enterForeground()
        if let cacheDirectory = FileManager.default.urls(for: .cachesDirectory, in: .userDomainMask).first {
            _ = session.configureImageCache(cacheDirectory: cacheDirectory.path)
        }
        let settings = HistorySettingsStore.load()
        historyModeKey = settings.mode
        historyLimit = settings.limit
        session.setHistoryMode(mode: nativeMode(settings.mode))
        _ = session.setHistoryLimit(limit: UInt32(settings.limit))
        connectListener()
        updateSnapshot()
        refreshTask = Task { [weak self] in
            while !Task.isCancelled {
                self?.connectListener()
                self?.updateSnapshot()
                try? await Task.sleep(nanoseconds: 2_000_000_000)
            }
        }
    }

    private func connectListener() {
        guard active else { return }
        if identityBytes == nil { identityBytes = DeviceIdentityStore.loadOrCreate() }
        guard let identity = identityBytes else {
            connectionStatusKey = "identity_unavailable"
            return
        }
        fingerprint = session.identityFingerprint(identityBytes: identity)
        guard let address = TailnetAddress.currentIPv4() else {
            connectionStatusKey = "tailscale_unavailable"
            return
        }
        if listenerAddress == address {
            connectionStatusKey = "waiting_for_devices"
            return
        }
        if !permissionsRestored {
            for (id, directions) in DevicePolicyStore.load() {
                guard session.restoreDevicePolicy(peerId: id, send: directions.send, receive: directions.receive, text: directions.text, image: directions.image) else {
                    connectionStatusKey = "listener_unavailable"
                    return
                }
            }
            permissionsRestored = true
        }
        let error = session.startListener(
            identityBytes: identity,
            tailscaleIp: address,
            name: UIDevice.current.name
        )
        if error.isEmpty { listenerAddress = address }
        connectionStatusKey = error.isEmpty ? "waiting_for_devices" : "listener_unavailable"
    }

    func enterBackground() {
        active = false
        listenerAddress = nil
        refreshTask?.cancel()
        refreshTask = nil
        session.enterBackground()
        draft = nil
        imageDraft = nil
        updateSnapshot()
    }

    private func updateSnapshot() {
        historyRows = session.historyRows()
        historyCount = UInt32(historyRows.count)
        deviceRows = session.deviceRows()
        transferRows = session.transferRows()
        if !sendEvents.isEmpty {
            sendStatusKey = Self.sendStatus(transferRows.filter { sendEvents.contains($0.eventKey) }.map(\.state))
        }
        connectedPeerCount = session.connectedPeersCount()
        if connectedPeerCount > 0 { connectionStatusKey = "devices_connected" }
        else if connectionStatusKey == "devices_connected" { connectionStatusKey = "waiting_for_devices" }
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

    func refreshHistory() {
        if !session.requestHistoryRefresh() { settingsStatusKey = "refresh_unavailable" }
        else { settingsStatusKey = nil }
        updateSnapshot()
    }

    func setContentTypes(for row: MobileDeviceRow, text: Bool? = nil, image: Bool? = nil) {
        let value = DevicePolicyStore.load()[row.id] ?? DeviceDirections(send: row.send, receive: row.receive, text: row.text, image: row.image)
        guard DevicePolicyStore.save(id: row.id, send: value.send, receive: value.receive, text: text ?? value.text, image: image ?? value.image),
              session.restoreDevicePolicy(peerId: row.id, send: value.send, receive: value.receive, text: text ?? value.text, image: image ?? value.image) else {
            settingsStatusKey = "settings_unavailable"; return
        }
        settingsStatusKey = nil
        _ = session.requestHistoryRefresh()
        updateSnapshot()
    }

    func setReceive(_ enabled: Bool, for row: MobileDeviceRow) {
        if DevicePolicyStore.save(id: row.id, send: row.send, receive: enabled) {
            _ = session.setReceive(peerId: row.id, enabled: enabled)
            settingsStatusKey = nil
            _ = session.requestHistoryRefresh()
        } else {
            settingsStatusKey = "settings_unavailable"
        }
        updateSnapshot()
    }

    func setSend(_ enabled: Bool, for row: MobileDeviceRow) {
        if DevicePolicyStore.save(id: row.id, send: enabled, receive: row.receive) {
            _ = session.setSend(peerId: row.id, enabled: enabled)
            settingsStatusKey = nil
        } else {
            settingsStatusKey = "settings_unavailable"
        }
        updateSnapshot()
    }

    private func nativeMode(_ value: String) -> MobileHistoryMode {
        switch value {
        case "off": .off
        case "status": .status
        default: .content
        }
    }

    func setHistoryMode(_ mode: String) {
        let value = HistorySettings(mode: mode, limit: historyLimit)
        guard HistorySettingsStore.save(value) else { settingsStatusKey = "settings_unavailable"; return }
        session.setHistoryMode(mode: nativeMode(mode))
        historyModeKey = mode
        updateSnapshot()
    }

    func setHistoryLimit(_ limit: Int) {
        let value = HistorySettings(mode: historyModeKey, limit: limit)
        guard HistorySettingsStore.save(value) else { settingsStatusKey = "settings_unavailable"; return }
        guard session.setHistoryLimit(limit: UInt32(limit)) else { return }
        historyLimit = limit
        updateSnapshot()
    }

    var sendTargets: [MobileDeviceRow] {
        deviceRows.filter { $0.online && $0.send && (imageDraft != nil ? $0.image : draft != nil ? $0.text : $0.text || $0.image) }
    }

    var allowedSendCount: Int {
        sendTargets.count
    }

    func dismissDraft() {
        draft = nil
        imageDraft = nil
        sendStatusKey = nil
        sendEvents = []
    }

    func sendDraft() {
        let previous = Set(session.transferRows().map(\.eventKey))
        let queued: UInt32
        if let imageDraft {
            queued = session.sendImage(pngBytes: imageDraft)
        } else if let draft {
            queued = session.sendText(text: draft)
        } else { return }
        sendStatusKey = queued > 0 ? "send_queued" : "no_send_targets"
        sendEvents = queued > 0 ? Set(session.transferRows().map(\.eventKey)).subtracting(previous) : []
        if queued > 0 { self.draft = nil; imageDraft = nil }
        updateSnapshot()
    }

    func resend(_ row: MobileHistoryRow) {
        let previous = Set(session.transferRows().map(\.eventKey))
        let queued = session.resendLocal(key: row.eventKey)
        sendStatusKey = queued > 0 ? "send_queued" : "no_send_targets"
        sendEvents = queued > 0 ? Set(session.transferRows().map(\.eventKey)).subtracting(previous) : []
        updateSnapshot()
    }

    func clearHistory() {
        session.clearHistory()
        draft = nil
        imageDraft = nil
        copyStatusKey = nil
        sendStatusKey = nil
        sendEvents = []
        updateSnapshot()
    }

    func importText(_ text: String) {
        guard !text.isEmpty, text.utf8.count <= 1_048_576, !text.contains("\0") else {
            sendStatusKey = "unsupported_text"; return
        }
        sendStatusKey = nil
        sendEvents = []
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
