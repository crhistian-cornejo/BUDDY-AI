import Foundation
import Security

/// A paired computer. Its keys live in the Keychain (see `Vault`), never here.
struct Machine: Codable, Identifiable, Equatable {
    /// The room at the relay: also this pairing's name for its keys.
    var id: String
    var name: String
    var platform: String
    var relay: String
    var desktopPublic: Data
}

/// The Keychain: this device only, readable while the phone is unlocked. Nothing here goes to iCloud or backups.
enum Vault {
    private static let service = "io.github.crhistian-cornejo.buddy.ios"

    static func set(_ value: Data, _ name: String) {
        delete(name)
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: name,
            kSecValueData as String: value, kSecAttrAccessible as String: kSecAttrAccessibleWhenUnlockedThisDeviceOnly,
        ]
        SecItemAdd(query as CFDictionary, nil)
    }

    static func get(_ name: String) -> Data? {
        let query: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: name,
            kSecReturnData as String: true, kSecMatchLimit as String: kSecMatchLimitOne,
        ]
        var found: AnyObject?
        return SecItemCopyMatching(query as CFDictionary, &found) == errSecSuccess ? found as? Data : nil
    }

    static func delete(_ name: String) {
        let query: [String: Any] = [kSecClass as String: kSecClassGenericPassword, kSecAttrService as String: service, kSecAttrAccount as String: name]
        SecItemDelete(query as CFDictionary)
    }
}

extension Machine {
    var privateKey: Data? { Vault.get("\(id).private") }
    var roomKey: String? { Vault.get("\(id).room").flatMap { String(data: $0, encoding: .utf8) } }

    /// The paired machines, kept between launches (names and addresses only).
    static func load() -> [Machine] {
        guard let data = UserDefaults.standard.data(forKey: "machines") else { return [] }
        return (try? JSONDecoder().decode([Machine].self, from: data)) ?? []
    }

    static func save(_ machines: [Machine]) {
        UserDefaults.standard.set(try? JSONEncoder().encode(machines), forKey: "machines")
    }

    /// Forgets this machine on the phone: its keys go with it.
    func forget() {
        for name in ["private", "room", "push"] { Vault.delete("\(id).\(name)") }
    }
}
