import SwiftUI
import ImageIO
import UIKit
import UniformTypeIdentifiers

private enum Brand {
    static let blue = Color(red: 43 / 255, green: 98 / 255, blue: 217 / 255)
    static let panel = Color(uiColor: UIColor { traits in
        traits.userInterfaceStyle == .dark
            ? UIColor(red: 40 / 255, green: 56 / 255, blue: 94 / 255, alpha: 1)
            : UIColor(red: 227 / 255, green: 235 / 255, blue: 255 / 255, alpha: 1)
    })
}

struct RootView: View {
    @AppStorage("ui-language") private var language = ""
    var body: some View {
        TabView {
            HistoryView().tabItem { Label("home", systemImage: "house") }
            DevicesView().tabItem { Label("devices", systemImage: "laptopcomputer") }
            SettingsView().tabItem { Label("settings", systemImage: "slider.horizontal.3") }
        }
        .tint(Brand.blue)
        .environment(\.locale, language.isEmpty ? Locale.current : Locale(identifier: language))
    }
}

private struct HistoryView: View {
    @EnvironmentObject private var state: AppState
    @State private var filter = 0

    private var filtered: [MobileHistoryRow] {
        state.historyRows.filter { filter == 0 || filter == 1 && $0.isLocal || filter == 2 && $0.kind == .image }
    }
    private var checked: UInt64? {
        state.deviceRows.filter { $0.receive && $0.checkedAtMs > 0 }.map(\.checkedAtMs).min()
    }
    var body: some View {
        NavigationStack {
            ScrollView {
                LazyVStack(alignment: .leading, spacing: 14) {
                    HStack(spacing: 8) {
                        ForEach(Array(["all", "sent_from_phone", "images"].enumerated()), id: \.offset) { index, key in
                            Button { filter = index } label: {
                                Text(LocalizedStringKey(key)).font(.subheadline)
                                    .padding(.horizontal, 14).padding(.vertical, 9)
                                    .foregroundStyle(filter == index ? Color(.systemBackground) : Color.primary)
                                    .background(filter == index ? Color.primary : Color(.secondarySystemGroupedBackground), in: Capsule())
                            }
                        }
                    }
                    if state.connectedPeerCount == 0 {
                        VStack(alignment: .leading, spacing: 6) {
                            Text(LocalizedStringKey(state.connectionStatusKey)).font(.headline)
                            Text("connect_help").font(.subheadline).foregroundStyle(.secondary)
                        }.card()
                    }
                    if filtered.isEmpty {
                        EmptyState(symbol: "doc.on.doc", title: "history_empty", detail: "history_empty_help")
                            .frame(maxWidth: .infinity).padding(.vertical, 40)
                    }
                    ForEach(filtered, id: \.eventKey) { row in HistoryCard(row: row) }
                }
                .padding(.horizontal, 16).padding(.bottom, 16)
            }
            .background(Color(.systemGroupedBackground))
            .navigationTitle("history")
            .toolbar {
                ToolbarItem(placement: .navigationBarTrailing) {
                    HStack(spacing: 8) {
                    if let checked {
                        Text(String(format: localized("checked_time"), shortTime(checked)))
                            .font(.caption).foregroundStyle(.secondary)
                    }
                    Button { state.refreshHistory() } label: { Image(systemName: "arrow.clockwise") }.accessibilityLabel(Text("refresh"))
                    }
                }
            }
            .safeAreaInset(edge: .bottom, spacing: 0) { SendPanel() }
        }
    }
}

private struct HistoryCard: View {
    @EnvironmentObject private var state: AppState
    let row: MobileHistoryRow
    @State private var data = Data()
    @State private var expanded = false

    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            HStack(spacing: 5) {
                Text(row.isLocal ? localized("sent_from_phone") : row.sourceName)
                    .font(.subheadline.weight(.semibold)).lineLimit(1)
                Text(shortTime(row.copiedAtMs) + " · " + localized(row.kind == .text ? "text_item" : "image_item"))
                    .font(.caption).foregroundStyle(.secondary)
            }
            if !data.isEmpty && row.kind == .text {
                let text = String(data: data, encoding: .utf8) ?? ""
                Text(text).font(.body).lineLimit(expanded ? nil : 8).textSelection(.enabled)
                if text.count > 350 || text.filter({ $0 == "\n" }).count >= 8 {
                    Button(expanded ? "show_less" : "show_more") { expanded.toggle() }.font(.subheadline)
                }
            } else if !data.isEmpty {
                NavigationLink { ImageDetail(row: row, data: data) } label: {
                    ContentImage(data: data).frame(maxHeight: 260).clipShape(RoundedRectangle(cornerRadius: 8))
                }
            } else {
                Text(row.receiving ? "history_receiving" : "item_listed").font(.subheadline).foregroundStyle(.secondary)
            }
            if row.isLocal {
                let transfers = state.transferRows.filter { $0.eventKey == row.eventKey }
                if !transfers.isEmpty { Divider() }
                ForEach(transfers, id: \.targetId) { TransferRowView(row: $0) }
            }
            HStack {
                Button { state.copyToPhone(row) } label: { Label("copy_to_phone", systemImage: "doc.on.doc") }
                    .buttonStyle(.bordered).disabled(data.isEmpty)
                if row.isLocal {
                    Button("send_again") { state.resend(row) }.disabled(data.isEmpty || state.allowedSendCount == 0)
                        .font(.subheadline)
                }
            }
        }
        .card()
        .task(id: row.available) { data = row.available ? state.body(for: row) : Data() }
    }
}

private struct SendPanel: View {
    @EnvironmentObject private var state: AppState
    @State private var showTargets = false
    private var hasDraft: Bool { state.draft != nil || state.imageDraft != nil }
    var body: some View {
        VStack(alignment: .leading, spacing: 8) {
            if hasDraft {
                HStack(alignment: .top) {
                    VStack(alignment: .leading, spacing: 4) {
                        Text("ready_to_send").font(.caption.weight(.semibold))
                        if let draft = state.draft { Text(draft).lineLimit(2).font(.subheadline) }
                        if let data = state.imageDraft { ContentImage(data: data).frame(height: 60) }
                    }.frame(maxWidth: .infinity, alignment: .leading)
                    Button { state.dismissDraft() } label: { Image(systemName: "xmark") }
                        .frame(minWidth: 44, minHeight: 44).accessibilityLabel(Text("cancel"))
                }
            }
            HStack {
                VStack(alignment: .leading, spacing: 4) {
                    Text("send_clipboard").font(.headline)
                    Button { showTargets = true } label: {
                        HStack(spacing: 4) {
                            Text(String(format: localized(hasDraft ? "to_devices" : "devices_can_receive"), state.allowedSendCount))
                            if hasDraft { Image(systemName: "chevron.down").font(.caption2) }
                        }.font(.caption)
                    }.foregroundStyle(.secondary)
                }.frame(maxWidth: .infinity, alignment: .leading)
                Group {
                    if hasDraft {
                        Button("send_to_devices") { state.sendDraft() }.disabled(state.allowedSendCount == 0)
                    } else {
                        PasteButton(supportedContentTypes: [.plainText, .image]) { providers in
                            guard let provider = providers.first else { return }
                            if provider.hasItemConformingToTypeIdentifier(UTType.image.identifier) {
                                _ = provider.loadObject(ofClass: UIImage.self) { object, _ in
                                    guard let image = object as? UIImage else { return }
                                    Task { @MainActor in state.importImage(image) }
                                }
                            } else {
                                _ = provider.loadObject(ofClass: NSString.self) { object, _ in
                                    guard let text = object as? String else { return }
                                    Task { @MainActor in state.importText(text) }
                                }
                            }
                        }
                    }
                }.buttonStyle(.borderedProminent).controlSize(.large).frame(minWidth: 112, minHeight: 48)
            }
            if let status = state.sendStatusKey { Text(LocalizedStringKey(status)).font(.caption).foregroundStyle(.secondary) }
            if let status = state.copyStatusKey { Text(LocalizedStringKey(status)).font(.caption).foregroundStyle(.secondary) }
        }
        .padding(.horizontal, 16).padding(.vertical, 12)
        .background(Brand.panel, in: RoundedRectangle(cornerRadius: 20))
        .sheet(isPresented: $showTargets) {
            NavigationStack {
                List {
                    Section {
                        if state.allowedSendCount == 0 { Text("no_send_targets") }
                        ForEach(state.sendTargets, id: \.id) { Text($0.name) }
                    } footer: { Text("targets_help") }
                }
                .navigationTitle("send_targets").navigationBarTitleDisplayMode(.inline)
                .toolbar { ToolbarItem(placement: .confirmationAction) { Button("done") { showTargets = false } } }
            }.presentationDetents([.medium])
        }
    }
}

private struct TransferRowView: View {
    let row: MobileTransferRow
    var body: some View {
        HStack {
            Text(row.targetName)
            Spacer()
            Label(LocalizedStringKey(row.state.label), systemImage: row.state.symbol).foregroundStyle(.secondary)
        }.font(.caption)
    }
}
private extension MobileTransferState {
    var label: String {
        switch self {
        case .queued: "transfer_queued"
        case .sending: "transfer_sending"
        case .applied: "transfer_applied"
        case .failed: "transfer_failed"
        case .unknown: "transfer_unknown"
        }
    }
    var symbol: String {
        switch self {
        case .applied: "checkmark.circle"
        case .failed: "exclamationmark.circle"
        case .unknown: "questionmark.circle"
        default: "arrow.up.circle"
        }
    }
}

private struct ImageDetail: View {
    @EnvironmentObject private var state: AppState
    let row: MobileHistoryRow
    let data: Data
    var body: some View {
        ScrollView {
            VStack(spacing: 16) {
                ContentImage(data: data)
                Button { state.copyToPhone(row) } label: { Label("copy_to_phone", systemImage: "doc.on.doc") }
                    .buttonStyle(.borderedProminent)
                if let status = state.copyStatusKey { Text(LocalizedStringKey(status)).font(.caption) }
            }.padding(16)
        }.navigationTitle("image_item").navigationBarTitleDisplayMode(.inline)
    }
}

private struct ContentImage: View {
    let data: Data
    @State private var image: CGImage?
    var body: some View {
        Group {
            if let image { Image(decorative: image, scale: 1).resizable().scaledToFit() }
            else { Text("preview_unavailable").font(.caption).foregroundStyle(.secondary) }
        }
        .task(id: data) { image = await Task.detached(priority: .utility) { SafeImage.thumbnail(data) }.value }
    }
}
private enum SafeImage {
    static func thumbnail(_ data: Data) -> CGImage? {
        guard let source = CGImageSourceCreateWithData(data as CFData, nil),
              let properties = CGImageSourceCopyPropertiesAtIndex(source, 0, nil) as? [CFString: Any],
              let width = properties[kCGImagePropertyPixelWidth] as? Int,
              let height = properties[kCGImagePropertyPixelHeight] as? Int,
              width > 0, height > 0, width <= 16384, height <= 16384,
              width * height <= 16_777_216 else { return nil }
        let options: [CFString: Any] = [kCGImageSourceCreateThumbnailFromImageAlways: true,
            kCGImageSourceCreateThumbnailWithTransform: true, kCGImageSourceThumbnailMaxPixelSize: 1024]
        return CGImageSourceCreateThumbnailAtIndex(source, 0, options as CFDictionary)
    }
}

private struct DevicesView: View {
    @EnvironmentObject private var state: AppState
    var body: some View {
        NavigationStack {
            List {
                Section {
                    if state.deviceRows.isEmpty { Text("no_connected_devices") }
                    ForEach(state.deviceRows, id: \.id) { device in
                        NavigationLink { DeviceDetail(id: device.id) } label: {
                            HStack(spacing: 12) {
                                Image(systemName: "laptopcomputer").font(.title2).foregroundStyle(.secondary)
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(device.name).font(.headline)
                                    Text(device.online ? "device_online" : "device_offline").font(.caption)
                                    Text(LocalizedStringKey(device.historyActivity.label)).font(.caption).foregroundStyle(.secondary)
                                }
                            }.padding(.vertical, 6)
                        }
                    }
                } header: { Text("device_permissions_help") }
            }.navigationTitle("devices")
        }
    }
}

private struct DeviceDetail: View {
    @EnvironmentObject private var state: AppState
    let id: String
    @State private var confirmSend = false
    private var device: MobileDeviceRow? { state.deviceRows.first { $0.id == id } }
    var body: some View {
        Group {
            if let device {
                Form {
                    Section("connection") {
                        Label(device.online ? "device_online" : "device_offline", systemImage: device.online ? "checkmark.circle" : "circle")
                        Text(LocalizedStringKey(device.historyActivity.label)).font(.subheadline).foregroundStyle(.secondary)
                        Text(device.endpoint).font(.system(.caption, design: .monospaced))
                        if device.checkedAtMs > 0 {
                            Text(String(format: localized("history_checked"), shortTime(device.checkedAtMs))).font(.caption)
                        }
                        if device.historyPartial { Text("history_partial").font(.caption) }
                    }
                    Section { Toggle("receive_from_device", isOn: Binding(get: { device.receive }, set: { state.setReceive($0, for: device) }))
                    } header: { Text("receiving") } footer: { Text("receive_help") }
                    Section {
                        Toggle("send_to_device", isOn: Binding(get: { device.send }, set: { enabled in
                            if enabled { confirmSend = true } else { state.setSend(false, for: device) }
                        }))
                    } header: { Text("sending") } footer: { Text("send_help") }
                    Section("content_types") {
                        Toggle("text_item", isOn: Binding(get: { device.text }, set: { state.setContentTypes(for: device, text: $0) }))
                        Toggle("images", isOn: Binding(get: { device.image }, set: { state.setContentTypes(for: device, image: $0) }))
                        Text("content_types_help").font(.caption).foregroundStyle(.secondary)
                    }
                    Section("device_fingerprint") { Text(fingerprint(device.id)).font(.system(.caption, design: .monospaced)).textSelection(.enabled) }
                    if let key = state.settingsStatusKey { Text(LocalizedStringKey(key)) }
                }
                .navigationTitle(device.name).navigationBarTitleDisplayMode(.inline)
                .alert("confirm_device", isPresented: $confirmSend) {
                    Button("allow_sending") { state.setSend(true, for: device) }
                    Button("cancel", role: .cancel) {}
                } message: { Text(String(format: localized("fingerprint_help"), device.name) + "\n\n" + fingerprint(device.id)) }
            } else { EmptyState(symbol: "wifi.slash", title: "no_connected_devices", detail: "connect_help") }
        }
    }
}

private struct SettingsView: View {
    @EnvironmentObject private var state: AppState
    @State private var confirmClear = false
    @State private var limitDraft = "20"
    @AppStorage("ui-language") private var language = ""
    var body: some View {
        NavigationStack {
            Form {
                Section("this_phone") {
                    Text(UIDevice.current.name).font(.headline)
                    Text("device_fingerprint").font(.caption).foregroundStyle(.secondary)
                    Text(fingerprint(state.fingerprint)).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    Text("manual_clipboard_help").font(.caption).foregroundStyle(.secondary)
                }
                Section("history_mode") {
                    Picker("history_mode", selection: Binding(get: { state.historyModeKey }, set: { state.setHistoryMode($0) })) {
                        Text("history_content").tag("content")
                        Text("history_status").tag("status")
                        Text("history_off").tag("off")
                    }.pickerStyle(.segmented)
                    HStack {
                        TextField("history_limit", text: $limitDraft).keyboardType(.numberPad)
                        Button("apply_history_limit") {
                            if let limit = Int(limitDraft), (0...10_000).contains(limit) { state.setHistoryLimit(limit) }
                        }.disabled(Int(limitDraft).map { !(0...10_000).contains($0) } ?? true)
                    }
                    Text("history_limit_help").font(.caption).foregroundStyle(.secondary)
                    Button("clear_history", role: .destructive) { confirmClear = true }
                    if let status = state.settingsStatusKey { Text(LocalizedStringKey(status)).font(.caption) }
                }
                Section("language") {
                    Picker("language", selection: $language) {
                        Text("system_language").tag("")
                        Text(verbatim: "English").tag("en")
                        Text(verbatim: "Nederlands").tag("nl")
                        Text(verbatim: "Deutsch").tag("de")
                        Text(verbatim: "Français").tag("fr")
                    }
                }
                Section("about") {
                    Text(Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "")
                    LabeledContent("app_version", value: Bundle.main.object(forInfoDictionaryKey: "CFBundleShortVersionString") as? String ?? "—")
                }
            }.navigationTitle("settings")
                .onAppear { limitDraft = String(state.historyLimit) }
                .alert("clear_history", isPresented: $confirmClear) {
                    Button("clear", role: .destructive) { state.clearHistory() }
                    Button("cancel", role: .cancel) {}
                } message: { Text("clear_history_help") }
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
            Text(detail).font(.subheadline).foregroundStyle(.secondary).multilineTextAlignment(.center)
        }.padding(24)
    }
}
private extension View {
    func card() -> some View {
        self.frame(maxWidth: .infinity, alignment: .leading).padding(16)
            .background(Color(.secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 14))
    }
}
private func shortTime(_ value: UInt64) -> String {
    Date(timeIntervalSince1970: Double(value) / 1000).formatted(date: .omitted, time: .shortened)
}
private func fingerprint(_ id: String) -> String {
    stride(from: 0, to: id.count, by: 4).map { offset in
        let start = id.index(id.startIndex, offsetBy: offset)
        return String(id[start..<id.index(start, offsetBy: min(4, id.count - offset))])
    }.joined(separator: " ")
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

private func localized(_ key: String) -> String {
    let language = UserDefaults.standard.string(forKey: "ui-language") ?? ""
    let bundle = Bundle.main.path(forResource: language, ofType: "lproj").flatMap(Bundle.init(path:)) ?? Bundle.main
    return bundle.localizedString(forKey: key, value: nil, table: nil)
}
