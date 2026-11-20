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
    var searchStatus: String?
    private var searchCursor: String?
    private var searchAccount: String?
    var searchStatusText: String? {
        switch searchStatus {
        case "ready": "Keyword + meaning matches · newest days first"
        case "partial": "Keyword + meaning matches · older mail is still indexing"
        case "indexing": "Keyword results · meaning search is still indexing"
        case "disabled": "Keyword results · enable meaning search in desktop Settings"
        case "exact": "Exact search · phrases and exclusions respected"
        case "keyword": "Keyword matches · checking meaning matches"
        case "busy": "Keyword results · search is busy, try again shortly"
        case "unavailable": "Keyword results · meaning search is temporarily unavailable"
        default: nil
        }
    }
    var selected: MessageRef?
    var rows: [MailMessage] = []
    var loading = false
    var loadingMore = false
    var hasMore = false
    var online = false
    var error: String?
    var notice: String?
    var sessionPromptRef: MessageRef?
    var sessionPrompt = ""
    private var openingSessions = Set<MessageRef>()
    var pending = Set<String>()
    var composer: LocalDraft?
    var recoveredDraft: LocalDraft?
    var recoveredEdits: [String: LocalDraft] = [:]
    var draftLinkRequest = 0
    var pendingDraftLink: String?
    var draftLinkError: String?
    var resolvingDraftLink = false
    private var draftLinkGeneration = 0
    var review: MessageRef?
    var unavailableDraft: MessageRef?
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
        recoveredEdits = cache.load("draft-edits.json") ?? [:]
        rows = snapshot.lists[listKey] ?? []
    }
    func requestSession(for ref: MessageRef) {
        sessionPrompt = ""
        sessionPromptRef = ref
    }

    func openSession(for ref: MessageRef, prompt: String = "") async {
        guard openingSessions.insert(ref).inserted else { return }
        defer { openingSessions.remove(ref) }
        do {
            var body = ref.payload
            let message = prompt.trimmingCharacters(in: .whitespacesAndNewlines)
            if !message.isEmpty { body["prompt"] = message }
            _ = try await api.data("/api/claude-thread", body: body)
            notice = "Opened a Claude thread in T3 on the mini."
        } catch {
            notice = "Couldn’t confirm that the session opened. \(friendly(error))"
        }
    }

    func receiveDraftLink(_ url: URL) {
        guard let id = DraftLink.parse(url) else { return }
        draftLinkGeneration += 1
        pendingDraftLink = id; draftLinkError = nil; draftLinkRequest += 1
    }
    func dismissDraftLink() {
        draftLinkGeneration += 1
        pendingDraftLink = nil; draftLinkError = nil; resolvingDraftLink = false
    }
    func resolvePendingDraftLink() async {
        guard let id = pendingDraftLink else { return }
        draftLinkGeneration += 1; let token = draftLinkGeneration
        let selection = selected; let currentTab = tab; let currentAccount = account; let currentReview = review
        resolvingDraftLink = true; draftLinkError = nil
        defer { if token == draftLinkGeneration { resolvingDraftLink = false } }
        guard composer == nil, review == nil, !chatPresented else {
            draftLinkError = "Close the current compose, review or chat window, then try this draft link again."
            return
        }
        do {
            let ref: MessageRef = try await api.get("/api/draft-link", query: [.init(name: "id", value: id)])
            guard token == draftLinkGeneration, pendingDraftLink == id, !Task.isCancelled else { return }
            guard selected == selection, tab == currentTab, account == currentAccount, review == currentReview, composer == nil, !chatPresented else {
                draftLinkError = "Your selection changed. Try again when you’re ready to open the linked draft."
                return
            }
            pendingDraftLink = nil
            // Use the root-owned review sheet; list refreshes cannot replace this reference.
            review = ref
        } catch {
            guard token == draftLinkGeneration, !Task.isCancelled else { return }
            draftLinkError = friendly(error)
        }
    }
    func load(more: Bool = false) async {
        if more && (loading || loadingMore || !hasMore) { return }
        generation += 1; let token = generation; let key = listKey
        if more { loadingMore = true } else {
            loading = true; loadingMore = false
            if tab != .search || searchAccount != account {
                rows = snapshot.lists[key] ?? []
                searchStatus = nil
            }
            error = nil; searchCursor = nil
            if tab == .search { searchAccount = account } else { searchAccount = nil }
        }
        defer { if token == generation { loading = false; loadingMore = false } }
        if tab == .search && query.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty { rows = []; hasMore = false; return }
        do {
            let accounts: [MailAccount] = try await api.get("/api/accounts")
            guard token == generation, key == listKey, !Task.isCancelled else { return }
            snapshot.accounts = accounts
            var items = [URLQueryItem(name: "account", value: account)]
            if tab == .search {
                items += [.init(name: "q", value: query), .init(name: "client", value: api.searchSession), .init(name: "generation", value: String(token))]
            }
            else { items.append(.init(name: "view", value: tab == .inbox ? category : tab.rawValue)) }
            if tab == .search {
                do {
                    try await loadSearch(items: items, more: more, token: token, key: key)
                } catch MailError.searchBusy {
                    guard token == generation, key == listKey, !Task.isCancelled else { return }
                    searchStatus = "busy"
                }
                guard token == generation, key == listKey, !Task.isCancelled else { return }
                online = true; error = nil
                snapshot.lists[key] = rows; snapshot.savedAt = Date(); persist(); return
            }
            if more, let last = rows.last { items.append(.init(name: "before", value: last.date)) }
            let fetched: [MailMessage] = try await api.get(tab == .drafts ? "/api/drafts" : "/api/messages", query: items)
            guard token == generation, key == listKey, !Task.isCancelled else { return }
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
            guard token == generation, key == listKey, !Task.isCancelled else { return }
            online = false; self.error = friendly(error)
        }
    }
    private func searchPage(_ items: [URLQueryItem]) async throws -> SearchPage {
        do { return try await api.get("/api/search", query: items) }
        catch MailError.searchBusy {
            try await Task.sleep(for: .seconds(1))
            return try await api.get("/api/search", query: items)
        }
    }
    private func loadSearch(items: [URLQueryItem], more: Bool, token: Int, key: String) async throws {
        let page: SearchPage
        var append = more
        if more, let cursor = searchCursor {
            do { page = try await searchPage(items + [.init(name: "cursor", value: cursor)]) }
            catch MailError.searchExpired {
                // A ranked snapshot expiring is not a connection failure.
                // Refresh page one instead of retrying the same dead cursor.
                page = try await searchPage(items); append = false
            }
        } else {
            let keyword: SearchPage = try await searchPage(items + [.init(name: "mode", value: "keyword")])
            guard token == generation, key == listKey, !Task.isCancelled else { return }
            rows = keyword.results; searchStatus = "keyword"
            searchCursor = keyword.nextCursor; hasMore = keyword.nextCursor != nil
            do { page = try await searchPage(items) }
            catch {
                guard token == generation, key == listKey, !Task.isCancelled else { return }
                searchStatus = "unavailable"; return
            }
        }
        guard token == generation, key == listKey, !Task.isCancelled else { return }
        var seen = Set<String>()
        rows = (append ? rows + page.results : page.results).filter { seen.insert($0.id).inserted }
        searchCursor = page.nextCursor; hasMore = page.nextCursor != nil; searchStatus = page.semanticStatus
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
    func body(_ ref: MessageRef, images: Bool = false, automatic: Bool = false) async throws -> String {
        try await imageDocument(ref, images: images, automatic: automatic).html
    }
    func imageDocument(_ ref: MessageRef, images: Bool = false, automatic: Bool = false) async throws -> ImageDocument {
        let document = try await api.imageDocument(query: ref.query + (images ? [.init(name: "images", value: automatic ? "auto" : "1")] : []))
        // Never persist bodies after a remote-image opt-in.
        if !images { snapshot.html[ref.id] = document.html; persist() }
        return document
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
            // A cancelled request is not a failed one: the reader's
            // `.task(id:)` cancels its automatic mark-read when you leave the
            // message, often after the mini already applied it. Keep the
            // optimistic state and stay quiet; the next refresh reconciles.
            if Self.isCancellation(error) { return false }
            if listKey == key && generation == token { rows = previous }
            self.error = friendly(error); return false
        }
    }
    static func isCancellation(_ error: Error) -> Bool {
        if error is CancellationError { return true }
        if let url = error as? URLError, url.code == .cancelled { return true }
        return false
    }
    func newDraft(reply: MailMessage? = nil, all: Bool = false) {
        guard composer == nil else { return }
        if let recoveredDraft { composer = recoveredDraft; return }
        composer = reply.map { LocalDraft.reply(to: $0, all: all) } ?? .init(account: account.isEmpty ? snapshot.accounts.first(where: { !$0.hidden })?.id ?? snapshot.accounts.first?.id ?? "" : account)
    }
    func remember(_ draft: LocalDraft) {
        do {
            if let revision = draft.revision {
                var edits = recoveredEdits
                edits[revision.ref.id] = draft
                try cache.save(edits, name: "draft-edits.json")
                recoveredEdits = edits
            } else {
                try cache.save(draft, name: "draft.json")
                recoveredDraft = draft
            }
        } catch { self.error = "Couldn’t save changes on this phone. Keep the composer open and save to the mini." }
    }
    func clearDraft(_ draft: LocalDraft? = nil) {
        if let revision = draft?.revision {
            recoveredEdits.removeValue(forKey: revision.ref.id)
            do { try cache.save(recoveredEdits, name: "draft-edits.json") }
            catch { self.error = "Couldn’t clear the recovered draft edit on this phone." }
            if composer?.id == draft?.id { composer = nil }
        } else {
            cache.remove("draft.json")
            recoveredDraft = nil
            composer = nil
        }
    }
    func saveDraft(_ draft: LocalDraft) async throws -> DraftResult {
        guard draft.valid else { throw MailError.server("Check the account and recipient email addresses.") }
        var body: [String: Any] = ["account": draft.account, "to": LocalDraft.addresses(draft.to), "cc": LocalDraft.addresses(draft.cc), "bcc": LocalDraft.addresses(draft.bcc ?? ""), "subject": draft.subject, "body": draft.body]
        if let reply = draft.reply { body["reply_folder"] = reply.folder; body["reply_uid"] = reply.uid }
        let result: DraftResult
        if let revision = draft.revision {
            body["folder"] = revision.ref.folder
            body["uid"] = revision.ref.uid
            body["digest"] = revision.digest
            result = try await api.post("/api/draft-edit", body: body)
        } else {
            result = try await api.post("/api/draft", body: body)
        }
        clearDraft(draft); snapshot.lists = [:]; persist(); return result
    }
    func persist() {
        do { try cache.save(snapshot, name: "mail-v2.json") }
        catch { notice = "Mail is available now, but this phone couldn’t save an offline copy." }
    }
    func refreshAfterMissingDraft(_ ref: MessageRef) async {
        // The mini confirmed this exact server revision is gone. Invalidate
        // every saved list, keeping unrelated drafts and locally typed replies.
        generation += 1; loading = false; loadingMore = false
        rows.removeAll { $0.ref == ref }
        for key in snapshot.lists.keys { snapshot.lists[key]?.removeAll { $0.ref == ref } }
        snapshot.messages.removeValue(forKey: ref.id); snapshot.html.removeValue(forKey: ref.id)
        persist()
        await load()
        // A different selection made during refresh belongs to the user.
        if selected == ref { selected = nil }
        if review == ref { review = nil }
        unavailableDraft = ref
    }
    func clearCache() {
        snapshot = .init(accounts: snapshot.accounts); rows = []; selected = nil; cache.remove("mail-v2.json")
    }
    func friendly(_ error: Error) -> String {
        if Self.isCancellation(error) { return "The request was cancelled before the mini answered." }
        if error is URLError { return "Can’t reach the mini. Check Tailscale and that CXMail is running. Saved mail is still available." }
        return error.localizedDescription
    }
}
