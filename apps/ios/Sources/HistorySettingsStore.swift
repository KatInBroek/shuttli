import Foundation

struct HistorySettings: Codable {
    var mode: String
    var limit: Int
}

enum HistorySettingsStore {
    private static var url: URL? {
        guard var directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? directory.setResourceValues(values)
        return directory.appendingPathComponent("history-settings-v1.json")
    }

    static func load() -> HistorySettings {
        guard let url else { return HistorySettings(mode: "off", limit: 0) }
        guard FileManager.default.fileExists(atPath: url.path) else { return HistorySettings(mode: "content", limit: 20) }
        guard let bytes = try? Data(contentsOf: url), bytes.count < 1024,
              let value = try? JSONDecoder().decode(HistorySettings.self, from: bytes),
              ["off", "status", "content"].contains(value.mode), (0...10_000).contains(value.limit)
        else { return HistorySettings(mode: "off", limit: 0) }
        return value
    }

    static func save(_ value: HistorySettings) -> Bool {
        guard ["off", "status", "content"].contains(value.mode), (0...10_000).contains(value.limit),
              var url = url, let bytes = try? JSONEncoder().encode(value) else { return false }
        do {
            try bytes.write(to: url, options: [.atomic, .completeFileProtectionUnlessOpen])
            var values = URLResourceValues()
            values.isExcludedFromBackup = true
            try url.setResourceValues(values)
            return true
        } catch {
            try? FileManager.default.removeItem(at: url)
            return false
        }
    }
}
