import SwiftUI
import ImageIO
import UIKit
import UniformTypeIdentifiers

struct RootView: View {
    var body: some View {
        TabView {
            HomeView()
                .tabItem { Label("home", systemImage: "house") }
            HistoryView()
                .tabItem { Label("history", systemImage: "clock.arrow.circlepath") }
            DevicesView()
                .tabItem { Label("devices", systemImage: "desktopcomputer") }
            SettingsView()
                .tabItem { Label("settings", systemImage: "gearshape") }
        }
    }
}

private struct HomeView: View {
    @EnvironmentObject private var state: AppState

    var body: some View {
        NavigationStack {
            List {
                Section {
                    Label(LocalizedStringKey(state.connectionStatusKey), systemImage: state.connectedPeerCount > 0 ? "checkmark.circle" : "wifi.slash")
                    Text(LocalizedStringKey(state.connectedPeerCount > 0 ? "history_help" : "connect_help"))
                        .foregroundStyle(.secondary)
                    ForEach(state.deviceRows, id: \.id) { device in
                        Text(device.name + ": " + NSLocalizedString(device.historyActivity.label, comment: "")).font(.caption)
                    }
                }
                Section("send_from_phone") {
                    Text("paste_text").foregroundStyle(.secondary)
                    PasteButton(payloadType: String.self) { strings in
                        if let text = strings.first { state.importText(text) }
                    }
                    .buttonStyle(.borderedProminent)
                    Text("paste_image").foregroundStyle(.secondary)
                    PasteButton(supportedContentTypes: [.image]) { providers in
                        guard let provider = providers.first else { return }
                        _ = provider.loadObject(ofClass: UIImage.self) { object, _ in
                            guard let image = object as? UIImage else { return }
                            Task { @MainActor in state.importImage(image) }
                        }
                    }
                    .buttonStyle(.bordered)
                    if let draft = state.draft {
                        Text(draft)
                            .lineLimit(5)
                            .textSelection(.enabled)
                    }
                    if let image = state.imageDraft, let thumbnail = SafeImage.thumbnail(image) {
                        Image(decorative: thumbnail, scale: 1).resizable().scaledToFit().frame(maxHeight: 180)
                    }
                    if state.draft != nil || state.imageDraft != nil {
                        Button("send_to_devices") { state.sendDraft() }
                            .disabled(state.allowedSendCount == 0)
                    }
                    if let status = state.sendStatusKey {
                        Text(LocalizedStringKey(status)).foregroundStyle(.secondary)
                    }
                }
                Section("recent_history") {
                    if state.historyRows.isEmpty {
                        Text("history_empty").foregroundStyle(.secondary)
                    } else {
                        ForEach(Array(state.historyRows.prefix(3)), id: \.eventKey) { row in
                            HistoryRowLink(row: row)
                        }
                    }
                }
                if !state.transferRows.isEmpty {
                    Section("sent_from_phone") {
                        ForEach(state.transferRows.prefix(3), id: \.uniqueID) { transfer in
                            TransferRowView(row: transfer)
                        }
                    }
                }
            }
            .navigationTitle(Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "")
        }
    }
}

private struct HistoryView: View {
    @EnvironmentObject private var state: AppState
    @State private var filter = 0

    private var filtered: [MobileHistoryRow] {
        state.historyRows.filter { filter == 0 || (filter == 2) == $0.isLocal }
    }

    var body: some View {
        NavigationStack {
            List {
                Picker("history_filter", selection: $filter) {
                    Text("all").tag(0)
                    Text("from_devices").tag(1)
                    Text("sent_from_phone").tag(2)
                }
                .pickerStyle(.menu)
                if filtered.isEmpty {
                    Text("history_empty").foregroundStyle(.secondary)
                }
                ForEach(filtered, id: \.eventKey) { row in
                    HistoryRowLink(row: row)
                }
            }
            .navigationTitle("history")
        }
    }
}

private struct TransferRowView: View {
    let row: MobileTransferRow

    var body: some View {
        HStack {
            Text(row.targetName)
            Spacer()
            Text(LocalizedStringKey(statusKey)).foregroundStyle(.secondary)
        }
    }

    private var statusKey: String {
        switch row.state {
        case .queued: "transfer_queued"
        case .sending: "transfer_sending"
        case .applied: "transfer_applied"
        case .failed: "transfer_failed"
        case .unknown: "transfer_unknown"
        }
    }
}

private extension MobileTransferRow {
    var uniqueID: String { eventKey + targetId }
}

private struct HistoryRowLink: View {
    let row: MobileHistoryRow

    var body: some View {
        NavigationLink {
            HistoryDetailView(row: row)
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                Label(row.kind == .text ? "text_item" : "image_item", systemImage: row.kind == .text ? "text.alignleft" : "photo")
                Text(row.isLocal ? LocalizedStringKey("this_phone") : LocalizedStringKey(row.sourceName))
                    .font(.subheadline).foregroundStyle(.secondary)
                Text(Date(timeIntervalSince1970: Double(row.copiedAtMs) / 1000).formatted(date: .abbreviated, time: .shortened))
                    .font(.caption).foregroundStyle(.secondary)
                Text(row.available ? "item_available" : row.receiving ? "history_receiving" : "item_listed")
                    .font(.caption).foregroundStyle(.secondary)
            }
        }
    }
}

private struct HistoryDetailView: View {
    @EnvironmentObject private var state: AppState
    let row: MobileHistoryRow
    @State private var data = Data()

    var body: some View {
        List {
            Section {
                LabeledContent("source", value: row.isLocal ? String(localized: "this_phone") : row.sourceName)
                LabeledContent("size", value: ByteCountFormatter.string(fromByteCount: Int64(row.bytes), countStyle: .file))
            }
            Section("preview") {
                if !row.available || data.isEmpty {
                    Text("content_unavailable").foregroundStyle(.secondary)
                } else if row.kind == .text {
                    Text(String(data: data, encoding: .utf8) ?? "")
                        .textSelection(.enabled)
                } else if let image = SafeImage.thumbnail(data) {
                    Image(decorative: image, scale: 1)
                        .resizable()
                        .scaledToFit()
                } else {
                    Text("preview_unavailable").foregroundStyle(.secondary)
                }
            }
            Section {
                Button("copy_to_phone") { state.copyToPhone(row) }
                    .disabled(!row.available || data.isEmpty)
                if let status = state.copyStatusKey {
                    Text(LocalizedStringKey(status)).foregroundStyle(.secondary)
                }
            }
            if row.isLocal {
                Section("sent_from_phone") {
                    ForEach(state.transferRows.filter { $0.eventKey == row.eventKey }, id: \.targetId) { transfer in
                        TransferRowView(row: transfer)
                    }
                    Button("send_again") { state.resend(row) }
                        .disabled(!row.available || state.allowedSendCount == 0)
                    if let status = state.sendStatusKey {
                        Text(LocalizedStringKey(status)).foregroundStyle(.secondary)
                    }
                }
            }
        }
        .navigationTitle(row.kind == .text ? "text_item" : "image_item")
        .onAppear { data = state.body(for: row) }
    }
}

private enum SafeImage {
    static func thumbnail(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 8192, height <= 8192,
              width * height <= 16_777_216 else { return nil }
        let options: [CFString: Any] = [
            kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true,
            kCGImageSourceThumbnailMaxPixelSize: 1024,
        ]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}

private struct DevicesView: View {
    @EnvironmentObject private var state: AppState

    var body: some View {
        NavigationStack {
            Group {
                if state.deviceRows.isEmpty {
                    EmptyState(symbol: "desktopcomputer", title: "no_connected_devices", detail: "connect_help")
                } else {
                    List(state.deviceRows, id: \.id) { device in
                        Section(device.name) {
                            Label(device.online ? "device_online" : "device_offline", systemImage: device.online ? "checkmark.circle" : "circle")
                            Text(LocalizedStringKey(device.historyActivity.label)).foregroundStyle(.secondary)
                            if device.checkedAtMs > 0 {
                                Text(String(format: NSLocalizedString("history_checked", comment: ""), Date(timeIntervalSince1970: Double(device.checkedAtMs) / 1000).formatted(date: .abbreviated, time: .shortened))).font(.caption)
                            }
                            if device.historyPartial { Text("history_partial").font(.caption) }
                            Toggle("receive_from_device", isOn: Binding(
                                get: { state.deviceRows.first(where: { $0.id == device.id })?.receive ?? device.receive },
                                set: { state.setReceive($0, for: device) }
                            ))
                            Toggle("send_to_device", isOn: Binding(
                                get: { state.deviceRows.first(where: { $0.id == device.id })?.send ?? device.send },
                                set: { state.setSend($0, for: device) }
                            ))
                            Text("manual_send_only").foregroundStyle(.secondary)
                            if let status = state.settingsStatusKey {
                                Text(LocalizedStringKey(status)).foregroundStyle(.secondary)
                            }
                        }
                    }
                }
            }
            .navigationTitle("devices")
        }
    }
}

private struct EmptyState: View {
    let symbol: String
    let title: LocalizedStringKey
    let detail: LocalizedStringKey

    var body: some View {
        VStack(spacing: 14) {
            Image(systemName: symbol).font(.largeTitle).accessibilityHidden(true)
            Text(title).font(.headline)
            Text(detail).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }
        .padding(24)
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var confirmClear = false
    @State private var limitDraft = "20"

    var body: some View {
        NavigationStack {
            Form {
                Section("privacy") {
                    Text("manual_clipboard_help")
                }
                Section("history") {
                    Picker("history_mode", selection: Binding(
                        get: { state.historyModeKey },
                        set: { state.setHistoryMode($0) }
                    )) {
                        Text("history_off").tag("off")
                        Text("history_status").tag("status")
                        Text("history_content").tag("content")
                    }
                    TextField("history_limit", text: $limitDraft)
                        .keyboardType(.numberPad)
                    Button("apply_history_limit") {
                        if let limit = Int(limitDraft), (0...10_000).contains(limit) {
                            state.setHistoryLimit(limit)
                        }
                    }
                    Text("history_limit_help").foregroundStyle(.secondary)
                    if let status = state.settingsStatusKey {
                        Text(LocalizedStringKey(status)).foregroundStyle(.secondary)
                    }
                    Button("clear_history", role: .destructive) { confirmClear = true }
                }
                Section {
                    LabeledContent("app_version") {
                        Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—")
                    }
                }
            }
            .navigationTitle("settings")
            .onAppear { limitDraft = String(state.historyLimit) }
            .alert("clear_history", isPresented: $confirmClear) {
                Button("clear", role: .destructive) { state.clearHistory() }
                Button("cancel", role: .cancel) {}
            } message: {
                Text("clear_history_help")
            }
        }
    }
}

private extension MobileHistoryActivity {
    var label: String {
        switch self {
        case .waiting: "history_waiting"
        case .updating: "history_updating"
        case .receiving: "history_receiving"
        case .updated: "history_updated"
        case .denied: "history_denied"
        case .failed: "history_failed"
        case .paused: "history_paused"
        case .unavailable: "history_unavailable"
        }
    }
}
