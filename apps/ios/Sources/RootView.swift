import SwiftUI
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
                    Label("no_connected_devices", systemImage: "wifi.slash")
                    Text("connect_help")
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
                    Text("history_empty")
                        .foregroundStyle(.secondary)
                }
            }
            .navigationTitle(Bundle.main.object(forInfoDictionaryKey: "CFBundleDisplayName") as? String ?? "")
        }
    }
}

private struct HistoryView: View {
    var body: some View {
        NavigationStack {
            EmptyState(symbol: "clock", title: "history_empty", detail: "history_help")
                .navigationTitle("history")
        }
    }
}

private struct DevicesView: View {
    var body: some View {
        NavigationStack {
            EmptyState(symbol: "desktopcomputer", title: "no_connected_devices", detail: "connect_help")
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
            Image(systemName: symbol)
                .font(.largeTitle)
                .accessibilityHidden(true)
            Text(title)
                .font(.headline)
            Text(detail)
                .foregroundStyle(.secondary)
                .multilineTextAlignment(.center)
        }
        .padding(24)
    }
}

private struct SettingsView: View {
    var body: some View {
        NavigationStack {
            Form {
                Section("privacy") {
                    Text("manual_clipboard_help")
                }
            }
            .navigationTitle("settings")
        }
    }
}
