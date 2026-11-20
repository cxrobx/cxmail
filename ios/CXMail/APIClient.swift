import Foundation

enum MailError: LocalizedError {
    case configuration, server(String), unexpected, draftUnavailable, reviewExpired, searchExpired, searchBusy
    var errorDescription: String? {
        switch self {
        case .configuration: "The mini’s HTTPS address is missing. Rebuild with scripts/configure.sh."
        case .server(let message): message
        case .unexpected: "The mini returned an unexpected response."
        case .searchExpired: "Search expired. Search again to refresh the results."
        case .searchBusy: "Search is busy. Try again shortly."
        case .reviewExpired: "The draft review expired or the draft changed. Refresh before sending."
        case .draftUnavailable: "That draft changed or was removed. Choose its latest version from Drafts."
        }
    }
}

final class NoRedirects: NSObject, URLSessionTaskDelegate {
    func urlSession(_ session: URLSession, task: URLSessionTask, willPerformHTTPRedirection response: HTTPURLResponse,
                    newRequest request: URLRequest, completionHandler: @escaping (URLRequest?) -> Void) {
        completionHandler(nil)
    }
}

struct ImageDocument { var html: String; var imagesAllowed: Bool }

@MainActor
final class APIClient {
    let baseURL: URL?
    let fixture: Bool
    let searchSession = UUID().uuidString
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
        if path == "/api/search" { return 45 }
        if path == "/api/message" { return 210 }
        if ["/api/send", "/api/schedule", "/api/draft-preview", "/api/draft-edit", "/api/draft-attachment", "/api/body", "/api/image-trust"].contains(path) { return 150 }
        return path == "/api/chat/events" ? 35 : 100
    }
    func data(_ path: String, query: [URLQueryItem] = [], body: [String: Any]? = nil) async throws -> Data {
        try await transfer(path, query: query, body: body).0
    }
    private func transfer(_ path: String, query: [URLQueryItem], body: [String: Any]?) async throws -> (Data, HTTPURLResponse) {
        #if DEBUG
        if fixture {
            if path == "/api/draft-link" { try await Task.sleep(for: Fixtures.draftLinkDelay) }
            if path == "/api/search" { try await Task.sleep(for: Fixtures.searchDelay) }
            if path == "/api/action", body?["action"] as? String == "read",
               ProcessInfo.processInfo.arguments.contains("--slow-read") {
                try await Task.sleep(for: .seconds(30))
            }
            let data = try Fixtures.response(path, query: query, body: body)
            let mode = query.first { $0.name == "images" }?.value
            let allow = mode == "1" || (mode == "auto" && Fixtures.imageSenderTrusted && !ProcessInfo.processInfo.arguments.contains("--image-auth-failed"))
            var headers = ["X-CXMail-Images-Allowed": allow ? "1" : "0"]
            if path == "/api/draft-attachment" {
                headers["Content-Type"] = data.starts(with: "%PDF-".utf8) ? "application/pdf" : "application/octet-stream"
            }
            let response = HTTPURLResponse(url: URL(string: "https://fixture.example.ts.net")!, statusCode: 200, httpVersion: nil, headerFields: headers)!
            return (data, response)
        }
        #endif
        let request = try request(path, query: query, body: body)
        let (data, response) = try await session.data(for: request)
        guard let response = response as? HTTPURLResponse else { throw MailError.unexpected }
        guard (200..<300).contains(response.statusCode) else {
            let error = (try? JSONSerialization.jsonObject(with: data)) as? [String: Any]
            if response.statusCode == 410 && error?["code"] as? String == "search_expired" { throw MailError.searchExpired }
            if response.statusCode == 429 && error?["code"] as? String == "search_busy" { throw MailError.searchBusy }
            if response.statusCode == 409 && error?["code"] as? String == "review_expired" { throw MailError.reviewExpired }
            if response.statusCode == 410 && error?["code"] as? String == "draft_gone" { throw MailError.draftUnavailable }
            if response.statusCode == 403 { throw MailError.server("Access denied. Connect Tailscale using the mini’s allowed account.") }
            throw MailError.server(error?["error"] as? String ?? "The mini returned HTTP \(response.statusCode).")
        }
        return (data, response)
    }
    func get<T: Decodable>(_ path: String, query: [URLQueryItem] = [], as: T.Type = T.self) async throws -> T {
        try APIClient.decoder.decode(T.self, from: await data(path, query: query))
    }
    func post<T: Decodable>(_ path: String, body: [String: Any], as: T.Type = T.self) async throws -> T {
        try APIClient.decoder.decode(T.self, from: await data(path, body: body))
    }
    func draftAttachment(query: [URLQueryItem]) async throws -> Data {
        let (data, response) = try await transfer("/api/draft-attachment", query: query, body: nil)
        guard response.mimeType == "application/octet-stream" else { throw MailError.unexpected }
        return data
    }
    func officeAttachmentPreview(query: [URLQueryItem], format: String) async throws -> Data {
        let (data, response) = try await transfer("/api/draft-attachment",
            query: query + [.init(name: "render", value: format)], body: nil)
        guard response.mimeType == "application/pdf", data.starts(with: "%PDF-".utf8)
        else { throw MailError.unexpected }
        return data
    }
    func imageDocument(query: [URLQueryItem]) async throws -> ImageDocument {
        let (data, response) = try await transfer("/api/body", query: query, body: nil)
        guard let html = String(data: data, encoding: .utf8) else { throw MailError.unexpected }
        let header = response.value(forHTTPHeaderField: "X-CXMail-Images-Allowed")
        let manual = query.contains { $0.name == "images" && $0.value == "1" }
        return ImageDocument(html: html, imagesAllowed: header == "1" || (header == nil && manual))
    }
    func html(_ path: String, query: [URLQueryItem]) async throws -> String {
        guard let text = String(data: try await data(path, query: query), encoding: .utf8) else { throw MailError.unexpected }
        return text
    }
}
