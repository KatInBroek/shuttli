import SwiftUI
import ImageIO

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
                }
                Section("send_from_phone") {
                    PasteButton(payloadType: String.self) { strings in
                        if let text = strings.first { state.importText(text) }
                    }
                    .buttonStyle(.borderedProminent)
                    if let draft = state.draft {
                        Text(draft)
                            .lineLimit(5)
                            .textSelection(.enabled)
                        Button("send_to_devices") {}
                            .disabled(true)
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
            }
            .navigationTitle(Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "")
        }
    }
}

private struct HistoryView: View {
    @EnvironmentObject private var state: AppState

    var body: some View {
        NavigationStack {
            Group {
                if state.historyRows.isEmpty {
                    EmptyState(symbol: "clock", title: "history_empty", detail: "history_help")
                } else {
                    List(state.historyRows, id: \.eventKey) { row in
                        HistoryRowLink(row: row)
                    }
                }
            }
            .navigationTitle("history")
        }
    }
}

private struct HistoryRowLink: View {
    let row: MobileHistoryRow

    var body: some View {
        NavigationLink {
            HistoryDetailView(row: row)
        } label: {
            VStack(alignment: .leading, spacing: 4) {
                Label(row.kind == .text ? "text_item" : "image_item", systemImage: row.kind == .text ? "text.alignleft" : "photo")
                Text(row.sourceName).font(.subheadline).foregroundStyle(.secondary)
                Text(Date(timeIntervalSince1970: Double(row.copiedAtMs) / 1000).formatted(date: .abbreviated, time: .shortened))
                    .font(.caption).foregroundStyle(.secondary)
                Text(row.available ? "item_available" : "item_listed")
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
                LabeledContent("source", value: row.sourceName)
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
                            Toggle("receive_from_device", isOn: Binding(
                                get: { state.deviceRows.first(where: { $0.id == device.id })?.receive ?? device.receive },
                                set: { state.setReceive($0, for: device) }
                            ))
                            Text("manual_send_only").foregroundStyle(.secondary)
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

    var body: some View {
        NavigationStack {
            Form {
                Section("privacy") {
                    Text("manual_clipboard_help")
                }
                Section("history") {
                    Button("clear_history", role: .destructive) { confirmClear = true }
                }
            }
            .navigationTitle("settings")
            .alert("clear_history", isPresented: $confirmClear) {
                Button("clear", role: .destructive) { state.clearHistory() }
                Button("cancel", role: .cancel) {}
            } message: {
                Text("clear_history_help")
            }
        }
    }
}
