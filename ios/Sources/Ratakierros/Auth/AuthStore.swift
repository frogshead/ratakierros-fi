import Foundation
import Observation
import Security

@Observable
final class AuthStore {
    private(set) var token: String?
    private(set) var displayName: String?

    private let service = "fi.ratakierros.app"
    private let tokenAccount = "jwt"
    private let nameAccount = "display_name"

    init() {
        self.token = readKeychain(account: tokenAccount).flatMap { String(data: $0, encoding: .utf8) }
        self.displayName = readKeychain(account: nameAccount).flatMap { String(data: $0, encoding: .utf8) }
    }

    var isLoggedIn: Bool { token != nil }

    func setSession(token: String, displayName: String) {
        self.token = token
        self.displayName = displayName
        writeKeychain(account: tokenAccount, data: Data(token.utf8))
        writeKeychain(account: nameAccount, data: Data(displayName.utf8))
    }

    func signOut() {
        self.token = nil
        self.displayName = nil
        deleteKeychain(account: tokenAccount)
        deleteKeychain(account: nameAccount)
    }

    // MARK: - Keychain helpers

    private func writeKeychain(account: String, data: Data) {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        SecItemDelete(q as CFDictionary)
        var add = q
        add[kSecValueData as String] = data
        add[kSecAttrAccessible as String] = kSecAttrAccessibleAfterFirstUnlock
        SecItemAdd(add as CFDictionary, nil)
    }

    private func readKeychain(account: String) -> Data? {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account,
            kSecReturnData as String: true,
            kSecMatchLimit as String: kSecMatchLimitOne
        ]
        var item: CFTypeRef?
        let status = SecItemCopyMatching(q as CFDictionary, &item)
        guard status == errSecSuccess, let data = item as? Data else { return nil }
        return data
    }

    private func deleteKeychain(account: String) {
        let q: [String: Any] = [
            kSecClass as String: kSecClassGenericPassword,
            kSecAttrService as String: service,
            kSecAttrAccount as String: account
        ]
        SecItemDelete(q as CFDictionary)
    }
}
