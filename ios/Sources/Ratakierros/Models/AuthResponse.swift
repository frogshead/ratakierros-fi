import Foundation

struct AuthResponse: Codable {
    let token: String
    let userId: Int64
    let displayName: String

    enum CodingKeys: String, CodingKey {
        case token
        case userId = "user_id"
        case displayName = "display_name"
    }
}

struct LoginRequest: Codable {
    let email: String
    let password: String
}

struct RegisterRequest: Codable {
    let email: String
    let displayName: String
    let password: String

    enum CodingKeys: String, CodingKey {
        case email
        case displayName = "display_name"
        case password
    }
}

struct APIErrorPayload: Codable {
    let error: String?
}
