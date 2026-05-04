import Foundation

enum APIError: LocalizedError {
    case badStatus(Int, String?)
    case decoding(Error)
    case transport(Error)
    case unauthorized

    var errorDescription: String? {
        switch self {
        case .badStatus(_, let body): return body ?? "Pyyntö epäonnistui."
        case .decoding: return "Vastauksen lukeminen epäonnistui."
        case .transport(let e): return e.localizedDescription
        case .unauthorized: return "Kirjautuminen vanhentunut."
        }
    }
}

actor APIClient {
    private let baseURL: URL
    private let session: URLSession
    private let decoder: JSONDecoder
    private let encoder: JSONEncoder

    init(baseURL: URL? = nil, session: URLSession = .shared) {
        if let baseURL {
            self.baseURL = baseURL
        } else {
            let str = (Bundle.main.object(forInfoDictionaryKey: "API_BASE") as? String)
                ?? "https://ratakierros.fi"
            self.baseURL = URL(string: str)!
        }
        self.session = session
        self.decoder = JSONDecoder()
        self.encoder = JSONEncoder()
    }

    // MARK: - Public

    func tracks(lat: Double?, lon: Double?, query: String? = nil) async throws -> [Track] {
        var comps = URLComponents(url: baseURL.appendingPathComponent("/api/tracks"),
                                  resolvingAgainstBaseURL: false)!
        var items: [URLQueryItem] = []
        if let lat { items.append(URLQueryItem(name: "lat", value: String(lat))) }
        if let lon { items.append(URLQueryItem(name: "lon", value: String(lon))) }
        if let query, !query.isEmpty { items.append(URLQueryItem(name: "q", value: query)) }
        if !items.isEmpty { comps.queryItems = items }
        return try await get(comps.url!, token: nil)
    }

    func records(trackId: Int64, token: String?) async throws -> RecordsResponse {
        let url = baseURL.appendingPathComponent("/api/tracks/\(trackId)/records")
        return try await get(url, token: token)
    }

    func logRun(trackId: Int64, seconds: Double, token: String) async throws {
        let url = baseURL.appendingPathComponent("/api/runs")
        let body = LogRunRequest(trackId: trackId, timeSeconds: seconds)
        let _: LogRunResponse = try await post(url, body: body, token: token)
    }

    func login(email: String, password: String) async throws -> AuthResponse {
        let url = baseURL.appendingPathComponent("/api/auth/login")
        return try await post(url, body: LoginRequest(email: email, password: password), token: nil)
    }

    func register(email: String, displayName: String, password: String) async throws -> AuthResponse {
        let url = baseURL.appendingPathComponent("/api/auth/register")
        let body = RegisterRequest(email: email, displayName: displayName, password: password)
        return try await post(url, body: body, token: nil)
    }

    // MARK: - Internal

    private func get<T: Decodable>(_ url: URL, token: String?) async throws -> T {
        var req = URLRequest(url: url)
        req.httpMethod = "GET"
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        return try await perform(req)
    }

    private func post<Body: Encodable, T: Decodable>(_ url: URL, body: Body, token: String?) async throws -> T {
        var req = URLRequest(url: url)
        req.httpMethod = "POST"
        req.setValue("application/json", forHTTPHeaderField: "Content-Type")
        if let token { req.setValue("Bearer \(token)", forHTTPHeaderField: "Authorization") }
        req.httpBody = try encoder.encode(body)
        return try await perform(req)
    }

    private func perform<T: Decodable>(_ req: URLRequest) async throws -> T {
        let (data, resp): (Data, URLResponse)
        do {
            (data, resp) = try await session.data(for: req)
        } catch {
            throw APIError.transport(error)
        }
        guard let http = resp as? HTTPURLResponse else {
            throw APIError.badStatus(0, nil)
        }
        if http.statusCode == 401 { throw APIError.unauthorized }
        guard (200..<300).contains(http.statusCode) else {
            let body = (try? decoder.decode(APIErrorPayload.self, from: data))?.error
                ?? String(data: data, encoding: .utf8)
            throw APIError.badStatus(http.statusCode, body)
        }
        do {
            return try decoder.decode(T.self, from: data)
        } catch {
            throw APIError.decoding(error)
        }
    }
}
