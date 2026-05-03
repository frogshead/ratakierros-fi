import Foundation

struct Track: Codable, Identifiable, Hashable {
    let id: Int64
    let lipasId: Int64?
    let name: String?
    let lat: Double
    let lon: Double
    let city: String?
    let suburb: String?
    let address: String?
    let postalCode: String?
    let surface: String?
    let trackLengthM: Int64?
    let lanes: Int64?
    let status: String?
    let typeCode: Int64?
    let distanceM: Double?
    let record: Double?

    enum CodingKeys: String, CodingKey {
        case id
        case lipasId = "lipas_id"
        case name
        case lat
        case lon
        case city
        case suburb
        case address
        case postalCode = "postal_code"
        case surface
        case trackLengthM = "track_length_m"
        case lanes
        case status
        case typeCode = "type_code"
        case distanceM = "distance_m"
        case record
    }

    var displayName: String { name ?? "—" }

    var distanceLabel: String? {
        guard let d = distanceM else { return nil }
        if d < 1000 { return String(format: "%.0f m", d) }
        return String(format: "%.1f km", d / 1000.0)
    }

    var recordLabel: String? {
        guard let r = record else { return nil }
        return formatSeconds(r)
    }
}

func formatSeconds(_ seconds: Double) -> String {
    let total = max(0, seconds)
    let minutes = Int(total) / 60
    let remainder = total - Double(minutes * 60)
    return String(format: "%d:%05.2f", minutes, remainder)
}
