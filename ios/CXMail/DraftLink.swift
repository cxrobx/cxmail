import Foundation

/// Navigation only: a link cannot change the phone's trusted server or approve sending.
enum DraftLink {
    static var configuredHost: String? {
        let host = Bundle.main.object(forInfoDictionaryKey: "CXMailLinkHost") as? String
        return host == "links.invalid" ? nil : host
    }
    static func parse(_ url: URL, host: String? = configuredHost) -> String? {
        guard let c = URLComponents(url: url, resolvingAgainstBaseURL: false),
              c.user == nil, c.password == nil, c.port == nil, c.query == nil, c.fragment == nil else { return nil }
        let raw: String
        if c.scheme == "cxmail", c.host == "draft" { raw = String(c.percentEncodedPath.dropFirst()) }
        else if c.scheme == "https", let host, c.host == host, c.percentEncodedPath.hasPrefix("/draft/") {
            raw = String(c.percentEncodedPath.dropFirst(7))
        } else { return nil }
        guard raw.range(of: "^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$", options: .regularExpression) != nil else { return nil }
        return raw
    }
}
