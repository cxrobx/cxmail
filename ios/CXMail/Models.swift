import Foundation

struct MailAccount: Codable, Identifiable, Hashable {
    let id: String
    let email: String
    let name: String?
    let color: String?
    let hidden: Bool
}

struct MailAddress: Codable, Hashable {
    var name: String?
    var email: String
    var display: String { name.flatMap { $0.isEmpty ? nil : $0 } ?? email }
}

struct MessageRef: Codable, Hashable, Identifiable {
    var account: String
    var folder: String
    var uid: Int
    var id: String { "\(account.utf8.count):\(account)\(folder.utf8.count):\(folder):\(uid)" }
    var query: [URLQueryItem] {
        [.init(name: "account", value: account), .init(name: "folder", value: folder), .init(name: "uid", value: String(uid))]
    }
    var payload: [String: Any] { ["account": account, "folder": folder, "uid": uid] }
}

struct MailMessage: Codable, Hashable, Identifiable {
    var account: String
    var folder: String
    var uid: Int
    var subject: String
    var fromName: String?
    var fromEmail: String?
    var date: String
    var snippet: String
    var read: Bool
    var starred: Bool
    var attachments: Bool
    var category: String?
    var recipients: [String]?
    var draft: Bool?
    var to: [MailAddress]?
    var cc: [MailAddress]?
    var cached: Bool?
    var hasHtml: Bool?
    var plainText: String?
    var attachmentNames: [String]?
    var thread: [MailMessage]?
    var accountEmail: String?
    var imagesTrusted: Bool?
    var ref: MessageRef { .init(account: account, folder: folder, uid: uid) }
    var id: String { ref.id }
    var sender: String { fromName.flatMap { $0.isEmpty ? nil : $0 } ?? fromEmail ?? "Unknown sender" }
    var timestamp: Date? {
        ISO8601DateFormatter().date(from: date) ?? Self.fractional.date(from: date)
    }
    private static let fractional: ISO8601DateFormatter = {
        let f = ISO8601DateFormatter(); f.formatOptions = [.withInternetDateTime, .withFractionalSeconds]; return f
    }()
}

struct MailFolder: Codable, Identifiable {
    var name: String
    var label: String
    var type: String?
    var id: String { name }
}

struct DraftPreview: Decodable {
    var accountId: String
    var folder: String
    var uid: Int
    var from: MailAddress
    var to: [MailAddress]
    var cc: [MailAddress]
    var bcc: [MailAddress]
    var subject: String
    var plainText: String?
    var attachments: [String]
    var sendable: Bool
    var blockedReason: String?
    var digest: String
    var ref: MessageRef { .init(account: accountId, folder: folder, uid: uid) }
}

struct DraftResult: Decodable { var ok: Bool; var result: String?; var draft: MessageRef? }
struct SendResult: Decodable { var ok: Bool; var draftRemoved: Bool; var note: String? }
struct OK: Decodable { var ok: Bool }

enum MailTab: String, CaseIterable, Identifiable, Codable {
    case inbox, unread, drafts, sent, search
    var id: String { rawValue }
    var title: String { rawValue.capitalized }
    var symbol: String {
        switch self { case .inbox: "tray"; case .unread: "envelope.badge"; case .drafts: "doc"; case .sent: "paperplane"; case .search: "magnifyingglass" }
    }
}

struct LocalDraft: Codable, Equatable, Identifiable {
    var id = UUID()
    var account = ""
    var to = ""
    var cc = ""
    var subject = ""
    var body = ""
    var reply: MessageRef?
    var hasContent: Bool { !to.isEmpty || !subject.isEmpty || !body.isEmpty }
    static func addresses(_ raw: String) -> [String] {
        raw.split(whereSeparator: { $0 == "," || $0 == ";" || $0.isWhitespace }).map(String.init)
    }
    var valid: Bool {
        let recipients = Self.addresses(to)
        return !account.isEmpty && !recipients.isEmpty && recipients.count <= 50 && Self.addresses(cc).count <= 50 && (recipients + Self.addresses(cc)).allSatisfy {
            $0.count < 320 &&
            $0.range(of: #"^[^\s<>@]+@[^\s<>@]+\.[^\s<>@]+$"#, options: .regularExpression) != nil
        } && subject.count <= 998
    }
    static func reply(to message: MailMessage, all: Bool) -> Self {
        let me = (message.accountEmail ?? "").lowercased()
        let sender = message.fromEmail ?? ""
        let direct = sender.lowercased() == me ? (message.to ?? []).map(\.email).filter { $0.lowercased() != me } : [sender]
        var seen = Set((direct + [me]).map { $0.lowercased() })
        let cc = all ? ((message.to ?? []) + (message.cc ?? [])).map(\.email).filter { seen.insert($0.lowercased()).inserted } : []
        return .init(account: message.account, to: direct.joined(separator: ", "), cc: cc.joined(separator: ", "), subject: message.subject.lowercased().hasPrefix("re:") ? message.subject : "Re: " + message.subject, reply: message.ref)
    }
}
