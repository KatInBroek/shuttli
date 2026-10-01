import Foundation

struct DeviceDirections: Codable {
    let send: Bool
    let receive: Bool
    let text: Bool
    let image: Bool

    init(send: Bool, receive: Bool, text: Bool = true, image: Bool = true) {
        self.send = send; self.receive = receive; self.text = text; self.image = image
    }
    private enum CodingKeys: String, CodingKey { case send, receive, text, image }
    init(from decoder: Decoder) throws {
        let source = try decoder.container(keyedBy: CodingKeys.self)
        send = try source.decode(Bool.self, forKey: .send)
        receive = try source.decode(Bool.self, forKey: .receive)
        text = try source.decodeIfPresent(Bool.self, forKey: .text) ?? true
        image = try source.decodeIfPresent(Bool.self, forKey: .image) ?? true
    }
}

enum DevicePolicyStore {
    private static var url: URL? {
        guard var directory = FileManager.default.urls(for: .applicationSupportDirectory, in: .userDomainMask).first else { return nil }
        try? FileManager.default.createDirectory(at: directory, withIntermediateDirectories: true)
        var values = URLResourceValues()
        values.isExcludedFromBackup = true
        try? directory.setResourceValues(values)
        return directory.appendingPathComponent("device-directions-v1.json")
    }

    static func load() -> [String: DeviceDirections] {
        guard let url, let data = try? Data(contentsOf: url), data.count <= 16_384,
              let value = try? JSONDecoder().decode([String: DeviceDirections].self, from: data) else { return [:] }
        return value
    }

    @discardableResult
    static func save(id: String, send: Bool, receive: Bool, text: Bool? = nil, image: Bool? = nil) -> Bool {
        guard id.count == 64, id.utf8.allSatisfy({ ($0 >= 48 && $0 <= 57) || ($0 >= 97 && $0 <= 102) }),
              var url = url else { return false }
        var current = load()
        current[id] = DeviceDirections(send: send, receive: receive, text: text ?? current[id]?.text ?? true, image: image ?? current[id]?.image ?? true)
        guard current.count <= 32, let bytes = try? JSONEncoder().encode(current) else { return false }
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
