import Foundation

struct LogRunRequest: Codable {
    let trackId: Int64
    let timeSeconds: Double

    enum CodingKeys: String, CodingKey {
        case trackId = "track_id"
        case timeSeconds = "time_seconds"
    }
}

struct LogRunResponse: Codable {
    let ok: Bool
}

struct RecordEntry: Codable, Identifiable, Hashable {
    let rank: Int
    let displayName: String
    let timeSeconds: Double
    let loggedAt: String

    var id: String { "\(rank)-\(displayName)-\(timeSeconds)" }

    enum CodingKeys: String, CodingKey {
        case rank
        case displayName = "display_name"
        case timeSeconds = "time_seconds"
        case loggedAt = "logged_at"
    }
}

struct RecordsResponse: Codable {
    let track: Track
    let records: [RecordEntry]
    let personalBest: Double?

    enum CodingKeys: String, CodingKey {
        case track
        case records
        case personalBest = "personal_best"
    }
}
