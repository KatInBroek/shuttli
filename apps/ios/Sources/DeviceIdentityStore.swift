import Foundation
import Security

enum DeviceIdentityStore {
    private static let service = Bundle.main.bundleIdentifier ?? "org.katinbroek.shuttli"
    private static let account = "device-identity-v1"

    static func loadOrCreate() -> Data? {
        if let existing = load() { return existing }
        let bytes = generateIdentityBytes()
        guard !bytes.isEmpty else { return nil }
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecAttrAccessible: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
            kSecValueData: bytes,
        ]
        let result = SecItemAdd(query as CFDictionary, nil)
        if result == errSecSuccess { return bytes }
        if result == errSecDuplicateItem { return load() }
        return nil
    }

    private static func load() -> Data? {
        let query: [CFString: Any] = [
            kSecClass: kSecClassGenericPassword,
            kSecAttrService: service,
            kSecAttrAccount: account,
            kSecReturnData: true,
            kSecMatchLimit: kSecMatchLimitOne,
        ]
        var result: CFTypeRef?
        guard SecItemCopyMatching(query as CFDictionary, &result) == errSecSuccess else { return nil }
        return result as? Data
    }
}
