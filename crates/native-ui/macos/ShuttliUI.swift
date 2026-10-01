import AppKit
import SwiftUI
import Foundation

struct Peer: Identifiable {
    let id: String; let name: String; let address: String; let online: Bool; let policy: [String: Any]
}
struct Entry: Identifiable {
    let id: Int; let direction: String; let state: String; let format: String
    let detail: String; let peer: String; let available: Bool; let bytes: Int
}
final class Model: ObservableObject {
    @Published var message = ""
    @Published var fingerprint = ""
    @Published var clipboard = ""
    @Published var settings: [String: Any] = [:]
    @Published var deviceCounts: [String: Int] = [:]
    @Published var peers: [Peer] = []
    @Published var entries: [Entry] = []
    @Published var offset = 0
    @Published var previewText: String? = nil
    @Published var previewImage: NSImage? = nil
    private let queue = DispatchQueue(label: "org.shuttli.ui.api")
    private var pending = 0
    let executable: String
    init(_ executable: String) { self.executable = executable }
    func applyStatus(_ answer: [String: Any]) {
        guard let status = answer["status"] as? [String: Any] else { return }
        fingerprint = status["device"] as? String ?? ""
        clipboard = status["clipboard"] as? String ?? ""
        settings = status["settings"] as? [String: Any] ?? [:]
        deviceCounts = status["devices"] as? [String: Int] ?? [:]
    }
    func call(_ args: [String], silent: Bool = false, complete: (([String: Any]) -> Void)? = nil) {
        if silent && pending > 0 { return }
        if pending >= 8 { message = "Please wait for the current operation"; return }
        pending += 1
        if !silent { message = "Working…" }
        queue.async {
            let process = Process()
            process.executableURL = URL(fileURLWithPath: self.executable)
            process.arguments = args + ["--json"]
            let pipe = Pipe(); process.standardOutput = pipe; process.standardError = FileHandle.nullDevice
            do {
                try process.run()
                let bytes = pipe.fileHandleForReading.readDataToEndOfFile()
                process.waitUntilExit()
                guard bytes.count <= 12 * 1024 * 1024,
                      let answer = try JSONSerialization.jsonObject(with: bytes) as? [String: Any] else { throw NSError(domain: "Invalid agent response", code: 1) }
                DispatchQueue.main.async {
                    self.pending -= 1
                    if answer["type"] as? String == "stopped" { NSApp.terminate(nil); return }
                    if !silent || answer["type"] as? String == "error" { self.message = answer["message"] as? String ?? (answer["status"] as? [String:Any])?["message"] as? String ?? "" }
                    if answer["type"] as? String != "error" { complete?(answer) }
                }
            } catch { DispatchQueue.main.async { self.pending -= 1; self.message = "Cannot reach the agent: \(error.localizedDescription)" } }
        }
    }
    func refresh() {
        call(["status"]) { a in
            self.applyStatus(a)
            let s = a["status"] as? [String: Any] ?? [:]
            if let error = s["last_error"] as? String { self.message = error }
        }
        call(["devices"]) { a in
            let policies = (a["settings"] as? [String: Any])?["peers"] as? [String: [String: Any]] ?? [:]
            self.peers = (a["devices"] as? [[String: Any]] ?? []).compactMap { d in
                guard let id = d["id"] as? String else { return nil }
                return Peer(id: id, name: d["name"] as? String ?? "", address: d["address"] as? String ?? "", online: d["online"] as? Bool ?? false, policy: policies[id] ?? [:])
            }
        }
        history()
    }
    func history(silent: Bool = false) {
        call(["history", "--offset", String(offset)], silent: silent) { a in self.entries = (a["entries"] as? [[String: Any]] ?? []).compactMap { e in
            guard let id = e["id"] as? Int else { return nil }
            return Entry(id: id, direction: e["direction"] as? String ?? "", state: e["state"] as? String ?? "", format: e["format"] as? String ?? "", detail: e["detail"] as? String ?? "", peer: e["peer"] as? String ?? "", available: e["available"] as? Bool ?? false, bytes: e["bytes"] as? Int ?? 0)
        } }
    }
    func preview(_ id: Int) {
        call(["history", "preview", String(id)]) { a in
            guard let encoded = a["base64"] as? String, let bytes = Data(base64Encoded: encoded) else { return }
            if a["format"] as? String == "text" { self.previewImage = nil; self.previewText = String(data: bytes, encoding: .utf8) }
            else { self.previewText = nil; self.previewImage = NSImage(data: bytes) }
        }
    }
}
struct Content: View {
    @ObservedObject var model: Model
    @State private var tab = 0
    @State private var historyLimit = ""
    var body: some View {
        VStack(alignment: .leading, spacing: 14) {
            HStack { Text(ProductBrand.name).font(.title2.bold()); Spacer(); Button("Send now") { model.call(["send"]) }; Button("Refresh") { model.refresh() } }
            Text(model.message).foregroundStyle(.secondary).lineLimit(3)
            Picker("Page", selection: $tab) { Text("Status").tag(0); Text("Devices").tag(1); Text("History").tag(2); Text("Settings").tag(3) }.pickerStyle(.segmented)
            ScrollView { VStack(alignment: .leading, spacing: 16) {
                if tab == 0 {
                    Text("Devices").font(.headline)
                    Text("Discovered devices: \(model.deviceCounts["discovered"] ?? 0)")
                    Text("↑ Allowed to send: \(model.deviceCounts["send"] ?? 0) · ↓ Allowed to receive: \(model.deviceCounts["receive"] ?? 0)")
                    Text("Counts include offline devices. Both the global and device switches must be on. These are local permissions; delivery also depends on the other device.").foregroundStyle(.secondary)
                    Text("Directions").font(.headline)
                    ForEach(["send", "receive"], id: \.self) { key in
                        Toggle(key.capitalized, isOn: Binding(get: { model.settings[key] as? Bool ?? false }, set: { on in model.call(["set", key, on ? "on" : "off"]) { _ in model.refresh() } }))
                    }
                    Text("Device fingerprint").font(.headline)
                    Text(model.fingerprint).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                    Text(model.clipboard)
                    Text("A confirmed delivery means the receiver accepted the content at that time. Later copies or history cleanup can replace it.").foregroundStyle(.secondary)
                } else if tab == 1 {
                    Text("Allow outgoing sync separately for each device. Verify its full fingerprint.")
                    ForEach(model.peers) { peer in VStack(alignment: .leading, spacing: 8) {
                        Text(peer.name).font(.headline); Text(peer.address + (peer.online ? " · Online" : " · Offline"))
                        Text(peer.id).font(.system(.caption, design: .monospaced)).textSelection(.enabled)
                        HStack { ForEach(["send", "receive", "text", "png", "quiet"], id: \.self) { key in
                            Toggle(key == "png" ? "Images" : key.capitalized, isOn: Binding(get: { peer.policy[key] as? Bool ?? ["receive", "text", "png"].contains(key) }, set: { on in model.call(["peer", peer.id, key, on ? "on" : "off"]) { _ in model.refresh() } })).toggleStyle(.checkbox)
                        } }
                        Divider()
                    } }
                } else if tab == 2 {
                    HStack { Button("Previous") { model.offset = max(0,model.offset-50); model.history() }.disabled(model.offset == 0); Button("Next") { model.offset += 50; model.history() }.disabled(model.entries.count < 50) }
                    Button("Clear history and image cache") { model.call(["history", "clear"]) { _ in model.previewText = nil; model.previewImage = nil; model.history() } }
                    ForEach(model.entries) { e in VStack(alignment: .leading, spacing: 6) {
                        Text("#\(e.id) · \(e.direction == "local" ? "Local copy" : e.direction) · \(e.direction == "local" ? "Copied locally" : e.state) · \(e.format == "png" ? "Image" : e.format.capitalized) · \(e.bytes) bytes").font(.headline)
                        Text(e.peer).font(.system(.caption, design: .monospaced))
                        Text(e.detail).foregroundStyle(.secondary)
                        HStack {
                            Button("Preview") { model.preview(e.id) }
                            Button("Copy") { model.call(["history", "copy", String(e.id)]) }
                            Button("Copy locally") { model.call(["history", "copy", String(e.id), "--local-only"]) }
                            Button("Resend") { model.call(["history", "resend", String(e.id)]) }
                        }.disabled(!e.available)
                        Divider()
                    } }
                    if let text = model.previewText { Text(text).textSelection(.enabled) }
                    if let image = model.previewImage { Image(nsImage: image).resizable().scaledToFit().frame(maxHeight: 360) }
                } else {
                    ForEach(["send", "receive", "automatic", "text", "png", "notifications"], id: \.self) { key in
                        Toggle(key == "png" ? "Images" : key.capitalized, isOn: Binding(get: { model.settings[key] as? Bool ?? false }, set: { on in model.call(["set", key, on ? "on" : "off"]) { _ in model.refresh() } }))
                    }
                    Picker("History", selection: Binding(get: { model.settings["history"] as? String ?? "status" }, set: { mode in model.call(["set", "history", mode]) { _ in model.refresh() } })) { Text("Off").tag("off"); Text("Status only").tag("status"); Text("Recent content").tag("content") }
                    Text("Text and the list stay in memory. Images use a temporary encrypted cache. Restart clears history; clearing or evicting items deletes their image files. No keychain is needed.").foregroundStyle(.secondary)
                    HStack {
                        Text("Recent items: \(model.settings["history_limit"] as? Int ?? 20)")
                        TextField("New limit (0–10000)", text: $historyLimit).frame(width: 180)
                        Button("Apply") { model.call(["set", "history-limit", historyLimit]) { _ in model.previewText = nil; model.previewImage = nil; model.refresh() } }
                    }
                    Button("Clear history and image cache") { model.call(["history", "clear"]) { _ in model.previewText = nil; model.previewImage = nil; model.history() } }
                    HStack { Button("Enable start at login") { model.call(["autostart", "on"]) }; Button("Disable start at login") { model.call(["autostart", "off"]) }; Button("Check status") { model.call(["autostart", "status"]) } }
                    Text("App version: \(ProductBrand.version)").foregroundStyle(.secondary).textSelection(.enabled)
                }
            }.frame(maxWidth: .infinity, alignment: .leading) }
        }.padding(20).frame(minWidth: 760, minHeight: 540).onAppear { model.refresh() }
    }
}
final class Delegate: NSObject, NSApplicationDelegate, NSWindowDelegate {
    var item: NSStatusItem!; var window: NSWindow!; var model: Model!
    func applicationDidFinishLaunching(_ notification: Notification) {
        guard CommandLine.arguments.count >= 2 else { NSApp.terminate(nil); return }
        model = Model(CommandLine.arguments[1])
        item = NSStatusBar.system.statusItem(withLength: NSStatusItem.squareLength)
        item.button?.image = NSImage(systemSymbolName: "doc.on.clipboard", accessibilityDescription: ProductBrand.name)
        let menu = NSMenu()
        let summary = NSMenuItem(title: ProductBrand.name, action: nil, keyEquivalent: "")
        menu.addItem(summary)
        menu.addItem(withTitle: "Open window", action: #selector(show), keyEquivalent: "")
        menu.addItem(withTitle: "Send clipboard now", action: #selector(send), keyEquivalent: "")
        menu.addItem(NSMenuItem.separator())
        menu.addItem(withTitle: "Quit " + ProductBrand.name, action: #selector(quit), keyEquivalent: "")
        for entry in menu.items { entry.target = self }
        item.menu = menu
        Timer.scheduledTimer(withTimeInterval: 2, repeats: true) { [weak self] _ in
            guard let self = self else { return }
            self.model.call(["status"], silent: true) { [weak self] answer in
                guard let self = self else { return }
                self.model.applyStatus(answer)
                summary.title = ProductBrand.name + " · ↑ \(self.model.deviceCounts["send"] ?? 0) · ↓ \(self.model.deviceCounts["receive"] ?? 0)"
                if self.window?.isVisible == true { self.model.history(silent: true) }
            }
        }
        if !CommandLine.arguments.contains("--background") { show() }
    }
    @objc func show() {
        if window == nil {
            window = NSWindow(contentRect: NSRect(x: 0, y: 0, width: 850, height: 640), styleMask: [.titled, .closable, .miniaturizable, .resizable], backing: .buffered, defer: false)
            window.title = ProductBrand.name; window.isReleasedWhenClosed = false; window.delegate = self
            window.contentView = NSHostingView(rootView: Content(model: model)); window.center()
        }
        window.makeKeyAndOrderFront(nil); NSApp.activate(ignoringOtherApps: true)
    }
    func windowWillClose(_ notification: Notification) { model.previewImage = nil; model.previewText = nil; model.entries = [] }
    @objc func send() { model.call(["send"]) }
    @objc func quit() { model.call(["quit"]) { _ in NSApp.terminate(nil) } }
}
let app = NSApplication.shared
let delegate = Delegate()
app.delegate = delegate
app.setActivationPolicy(.accessory)
app.run()
