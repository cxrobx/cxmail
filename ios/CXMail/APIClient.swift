import Foundation

enum MailError: LocalizedError {
    case configuration, server(String), unexpected
    var errorDescription: String? {
        switch self {
        case .configuration: "The mini’s HTTPS address is missing. Rebuild with scripts/configure.sh."
        case .server(let message): message
        case .unexpected: "The mini returned an unexpected response."
        }
    }
}

final class NoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

@MainActor
final class APIClient {
    let baseURL: URL?
    let fixture: Bool
    private let session: URLSession
    static var decoder: JSONDecoder {
        let decoder = JSONDecoder(); decoder.keyDecodingStrategy = .convertFromSnakeCase; return decoder
    }
    init(baseURL: URL?, fixture: Bool = false, session: URLSession? = nil) {
        self.baseURL = baseURL
        self.fixture = fixture
        let config = URLSessionConfiguration.ephemeral
        config.timeoutIntervalForRequest = 35
        config.timeoutIntervalForResource = 240
        config.urlCache = nil
        self.session = session ?? URLSession(configuration: config, delegate: NoRedirects(), delegateQueue: nil)
    }
    func request(_ path: String, query: [URLQueryItem] = [], body: [String: Any]? = nil) throws -> URLRequest {
        guard let baseURL, baseURL.scheme == "https", baseURL.host?.hasSuffix(".ts.net") == true,
              baseURL.user == nil, baseURL.password == nil,
              var components = URLComponents(url: baseURL, resolvingAgainstBaseURL: false) else { throw MailError.configuration }
        components.path = path
        components.queryItems = query.isEmpty ? nil : query
        guard let url = components.url else { throw MailError.configuration }
        var request = URLRequest(url: url)
        // The mini's preview/send subprocess can run for 120s; MCP calls for 90s.
        request.timeoutInterval = Self.timeout(for: path)
        request.cachePolicy = .reloadIgnoringLocalCacheData
        if let body {
            request.httpMethod = "POST"
            request.setValue("application/json", forHTTPHeaderField: "Content-Type")
            request.setValue("1", forHTTPHeaderField: "X-CXMail-Remote")
            request.httpBody = try JSONSerialization.data(withJSONObject: body)
        }
        return request
    }
    static func timeout(for path: String) -> TimeInterval {
        if path == "/api/message" { return 210 }
        if ["/api/send", "/api/schedule", "/api/draft-preview"].contains(path) { return 150 }
        return path == "/api/chat/events" ? 35 : 100
    }
    func data(_ path: String, query: [URLQueryItem] = [], body: [String: Any]? = nil) async throws -> Data {
        #if DEBUG
        if fixture { return try Fixtures.response(path, query: query, body: body) }
        #endif
        let request = try request(path, query: query, body: body)
        let (data, response) = try await session.data(for: request)
        guard let response = response as? HTTPURLResponse else { throw MailError.unexpected }
        guard (200..<300).contains(response.statusCode) else {
            let error = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            if response.statusCode == 403 { throw MailError.server("Access denied. Connect Tailscale using the mini’s allowed account.") }
            throw MailError.server(error?["error"] as? String ?? "The mini returned HTTP \(response.statusCode).")
        }
        return data
    }
    func get<T: Decodable>(_ path: String, query: [URLQueryItem] = [], as: T.Type = T.self) async throws -> T {
        try APIClient.decoder.decode(T.self, from: await data(path, query: query))
    }
    func post<T: Decodable>(_ path: String, body: [String: Any], as: T.Type = T.self) async throws -> T {
        try APIClient.decoder.decode(T.self, from: await data(path, body: body))
    }
    func html(_ path: String, query: [URLQueryItem]) async throws -> String {
        guard let text = String(data: try await data(path, query: query), encoding: .utf8) else { throw MailError.unexpected }
        return text
    }
}
