import SwiftUI

enum ChatContextScope: String { case mail, currentEmail }

@MainActor @Observable
final class MailStore {
    let api: APIClient
    let cache: MailCache
    var snapshot: MailSnapshot
    var tab: MailTab = .inbox
    var category = "primary"
    var account = ""
    var query = ""
    var selected: MessageRef?
    var rows: [MailMessage] = []
    var loading = false
    var loadingMore = false
    var hasMore = false
    var online = false
    var error: String?
    var notice: String?
    var pending = Set<String>()
    var composer: LocalDraft?
    var recoveredDraft: LocalDraft?
    var review: MessageRef?
    var chatPresented = false
    var chatDocked = false
    var chatCanDock = false
    var chatMailFraction: CGFloat = 0.5
    var chatContextScope: ChatContextScope = .mail
    var chatSeed: MessageRef? {
        get { chatContextScope == .currentEmail ? selected : nil }
        set {
            chatContextScope = newValue == nil ? .mail : .currentEmail
            if let newValue { selected = newValue }
        }
    }
    var chatContextTitle: String? {
        guard let ref = chatSeed else { return nil }
        return snapshot.messages[ref.id]?.subject ?? rows.first(where: { $0.ref == ref })?.subject
    }
    private var generation = 0
    var listKey: String { "\(tab.rawValue)|\(category)|\(account)|\(query)" }

    init(api: APIClient, cache: MailCache = .init()) {
        self.api = api; self.cache = cache
        snapshot = cache.load("mail-v2.json") ?? .init()
        // The first remote protocol could cache MCP metadata as body text.
        cache.remove("mail.json")
        recoveredDraft = cache.load("draft.json")
        rows = snapshot.lists[listKey] ?? []
    }
    func load(more: Bool = false) async {
        if more && (loading || loadingMore || !hasMore) { return }
        generation += 1; let token = generation; let key = listKey
        if more { loadingMore = true } else {
            loading = true; loadingMore = false
            rows = snapshot.lists[key] ?? []; error = nil
        }
        defer { if token == generation { loading = false; loadingMore = false } }
        if tab == .search && query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { rows = []; hasMore = false; return }
        do {
            let accounts: [MailAccount] = try await api.get("/api/accounts")
            guard token == generation, !Task.isCancelled else { return }
            snapshot.accounts = accounts
            var items = [URLQueryItem(name: "account", value: account)]
            if tab == .search { items.append(.init(name: "q", value: query)) }
            else { items.append(.init(name: "view", value: tab == .inbox ? category : tab.rawValue)) }
            if more, let last = rows.last { items.append(.init(name: "before", value: last.date)) }
            let fetched: [MailMessage] = try await api.get(tab == .drafts ? "/api/drafts" : "/api/messages", query: items)
            guard token == generation, !Task.isCancelled else { return }
            var seen = Set<String>()
            rows = (more ? rows + fetched : fetched).filter { seen.insert($0.id).inserted }
            hasMore = fetched.count == 50 && tab != .drafts
            online = true; error = nil
            print("CXMail: connected; \(accounts.count) accounts, \(rows.count) messages loaded")
            snapshot.lists[key] = rows
            // Bound disk use; old search results are disposable.
            if snapshot.lists.count > 20 { snapshot.lists = [key: rows] }
            snapshot.savedAt = Date(); persist()
        } catch {
            guard token == generation, !Task.isCancelled else { return }
            online = false; self.error = friendly(error)
        }
    }
    func message(_ ref: MessageRef) async throws -> MailMessage {
        let message: MailMessage = try await api.get("/api/message", query: ref.query)
        if message.bodyError == nil { snapshot.messages[ref.id] = message }
        if snapshot.messages.count > 100 {
            let keep = Set(snapshot.messages.values.sorted { $0.date > $1.date }.prefix(80).map(\.id))
            snapshot.messages = snapshot.messages.filter { keep.contains($0.key) }
            snapshot.html = snapshot.html.filter { keep.contains($0.key) }
        }
        online = true; persist(); return message
    }
    func body(_ ref: MessageRef, images: Bool = false) async throws -> String {
        let html = try await api.html("/api/body", query: ref.query + (images ? [.init(name: "images", value: "1")] : []))
        // Never persist bodies after a remote-image opt-in.
        if !images { snapshot.html[ref.id] = html; persist() }
        return html
    }
    func action(_ name: String, _ message: MailMessage, folder: String? = nil) async -> Bool {
        guard !pending.contains(message.id) else { return false }
        pending.insert(message.id); defer { pending.remove(message.id) }
        let previous = rows
        let key = listKey
        generation += 1
        let token = generation
        loading = false; loadingMore = false
        switch name {
        case "archive", "trash", "move": rows.removeAll { $0.id == message.id }
        default:
            if let index = rows.firstIndex(where: { $0.id == message.id }) {
                if name == "star" || name == "unstar" { rows[index].starred = name == "star" }
                if name == "read" || name == "unread" { rows[index].read = name == "read" }
            }
        }
        do {
            var body = message.ref.payload; body["action"] = name
            if let folder { body["to_folder"] = folder }
            let _: OK = try await api.post("/api/action", body: body)
            // Invalidate every list so switching tabs cannot resurrect moved mail.
            snapshot.lists = [:]
            if ["archive", "trash", "move"].contains(name) {
                snapshot.messages.removeValue(forKey: message.id); snapshot.html.removeValue(forKey: message.id)
            } else if var cached = snapshot.messages[message.id] {
                if name == "read" || name == "unread" { cached.read = name == "read" }
                if name == "star" || name == "unstar" { cached.starred = name == "star" }
                snapshot.messages[message.id] = cached
            }
            snapshot.lists[listKey] = rows; persist()
            if ["archive", "trash", "move"].contains(name), selected == message.ref { selected = nil }
            if name != "read" { notice = ["archive": "Archived", "trash": "Moved to Trash", "move": "Moved", "star": "Starred", "unstar": "Star removed", "unread": "Marked unread"][name] }
            return true
        } catch {
            if listKey == key && generation == token { rows = previous }
            self.error = friendly(error); return false
        }
    }
    func newDraft(reply: MailMessage? = nil, all: Bool = false) {
        guard composer == nil else { return }
        if let recoveredDraft { composer = recoveredDraft; return }
        composer = reply.map { LocalDraft.reply(to: $0, all: all) } ?? .init(account: account.isEmpty ? snapshot.accounts.first(where: { !$0.hidden })?.id ?? snapshot.accounts.first?.id ?? "" : account)
    }
    func remember(_ draft: LocalDraft) {
        do { try cache.save(draft, name: "draft.json"); recoveredDraft = draft }
        catch { self.error = "Couldn’t save the reply on this phone. Keep the composer open and save it to the mini." }
    }
    func clearDraft() { cache.remove("draft.json"); recoveredDraft = nil; composer = nil }
    func saveDraft(_ draft: LocalDraft) async throws -> DraftResult {
        guard draft.valid else { throw MailError.server("Check the account and recipient email addresses.") }
        var body: [String: Any] = ["account": draft.account, "to": LocalDraft.addresses(draft.to), "cc": LocalDraft.addresses(draft.cc), "subject": draft.subject, "body": draft.body]
        if let reply = draft.reply { body["reply_folder"] = reply.folder; body["reply_uid"] = reply.uid }
        let result: DraftResult = try await api.post("/api/draft", body: body)
        clearDraft(); snapshot.lists = [:]; persist(); return result
    }
    func persist() {
        do { try cache.save(snapshot, name: "mail-v2.json") }
        catch { notice = "Mail is available now, but this phone couldn’t save an offline copy." }
    }
    func clearCache() {
        snapshot = .init(accounts: snapshot.accounts); rows = []; selected = nil; cache.remove("mail-v2.json")
    }
    func friendly(_ error: Error) -> String {
        if error is URLError { return "Can’t reach the mini. Check Tailscale and that CXMail is running. Saved mail is still available." }
        return error.localizedDescription
    }
}
