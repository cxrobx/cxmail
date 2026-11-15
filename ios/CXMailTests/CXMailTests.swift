import XCTest
@testable import CXMail

final class FailingProtocol: URLProtocol {
    static var calls = 0
    static var responseCode = 502
    static var responseBody = #"{"error":"The mini is unavailable"}"#
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        Self.calls += 1
        let response = HTTPURLResponse(url: request.url!, statusCode: Self.responseCode, httpVersion: nil, headerFields: ["Content-Type": "application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(Self.responseBody.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() { }
}

@MainActor
final class CXMailTests: XCTestCase {
    func api() -> APIClient { APIClient(baseURL: URL(string: "https://mini.example.ts.net:18706"), fixture: true) }
    func cache() -> MailCache { MailCache(directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)) }
    func testDuoGeometryChoosesPushOverlayAndColumns() {
        XCTAssertFalse(DuoLayout.resolve(width: 382, height: 644, fold: nil).split)
        XCTAssertFalse(DuoLayout.resolve(width: 594, height: 432, fold: nil).split)
        XCTAssertFalse(DuoLayout.resolve(width: 669, height: 835, fold: CGRect(x: 0, y: 373.5, width: 669, height: 40)).split)
        XCTAssertTrue(DuoLayout.resolve(width: 669, height: 835, fold: nil).overlay)
        XCTAssertFalse(DuoLayout.resolve(width: 382, height: 644, fold: nil).overlay)
        let wide = DuoLayout.resolve(width: 867, height: 635, fold: CGRect(x: 455.5, y: 0, width: 40, height: 669))
        XCTAssertTrue(wide.split); XCTAssertEqual(wide.leading, 455.5); XCTAssertEqual(wide.gap, 40)
        let flat = DuoLayout.resolve(width: 867, height: 635, fold: nil)
        XCTAssertGreaterThan(867 - flat.leading - flat.gap, 500)
        // A transient previous-orientation region must not create an off-screen pane.
        XCTAssertTrue(DuoLayout.resolve(width: 867, height: 635, fold: CGRect(x: 900, y: 0, width: 40, height: 669)).split)
    }
    func testRequestsStayHTTPSOnTailnetAndPreserveOpaqueReferences() throws {
        let ref = MessageRef(account: "a&b", folder: "[Gmail]/All Mail & Friends", uid: 12)
        let request = try api().request("/api/message", query: ref.query)
        let query = URLComponents(url: request.url!, resolvingAgainstBaseURL: false)!.queryItems!
        XCTAssertEqual(query.first(where: { $0.name == "folder" })?.value, ref.folder)
        XCTAssertEqual(query.first(where: { $0.name == "account" })?.value, ref.account)
        for url in ["http://mini.example.ts.net", "https://example.com", "https://mini.example.ts.net.evil.com", "https://user:pass@mini.example.ts.net"] {
            XCTAssertThrowsError(try APIClient(baseURL: URL(string: url)).request("/api/accounts"))
        }
    }
    func testMutationCarriesCSRFHeaderAndExactReviewedDigest() throws {
        var body = MessageRef(account: "work", folder: "Drafts", uid: 20).payload
        body["digest"] = String(repeating: "b", count: 64)
        let request = try api().request("/api/send", body: body)
        XCTAssertEqual(request.httpMethod, "POST")
        XCTAssertEqual(request.value(forHTTPHeaderField: "X-CXMail-Remote"), "1")
        let sent = try JSONSerialization.jsonObject(with: request.httpBody!) as! [String: Any]
        XCTAssertEqual(sent["digest"] as? String, body["digest"] as? String)
    }
    func testReplyAllExcludesSelfAndDeduplicatesCaseInsensitively() throws {
        var message = try APIClient.decoder.decode(MailMessage.self, from: JSONSerialization.data(withJSONObject: Fixtures.message(detail: true)))
        message.cc = [.init(email: "JORDAN@example.org"), .init(email: "alex@example.org"), .init(email: "ALEX@example.org")]
        let draft = LocalDraft.reply(to: message, all: true)
        XCTAssertEqual(draft.to, "jordan@example.org")
        XCTAssertEqual(draft.cc, "alex@example.org")
        XCTAssertEqual(draft.reply, message.ref)
        XCTAssertEqual(draft.account, "work")
        message.fromEmail = "me@example.com"
        message.to = [.init(email: "jordan@example.org")]
        XCTAssertEqual(LocalDraft.reply(to: message, all: false).to, "jordan@example.org")
    }
    func testInvalidCcCannotBeSilentlyDropped() {
        var draft = LocalDraft(account: "work", to: "jordan@example.org", subject: "Hello", body: "Hi")
        XCTAssertTrue(draft.valid)
        draft.cc = "alex@example.org, wrong"
        XCTAssertFalse(draft.valid)
        draft.cc = ""; draft.to = ""; XCTAssertFalse(draft.valid)
        draft.to = Array(repeating: "jordan@example.org", count: 51).joined(separator: ",")
        XCTAssertFalse(draft.valid, "The mini caps each recipient list at 50; never truncate silently")
    }
    func testFailedMutationRollsBackAndDoesNotRetry() async throws {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [FailingProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://mini.example.ts.net"), session: URLSession(configuration: config))
        let store = MailStore(api: api, cache: cache())
        let message = try APIClient.decoder.decode(MailMessage.self, from: JSONSerialization.data(withJSONObject: Fixtures.message()))
        store.rows = [message]; FailingProtocol.calls = 0
        let succeeded = await store.action("archive", message)
        XCTAssertFalse(succeeded); XCTAssertEqual(store.rows, [message])
        XCTAssertNotNil(store.error); XCTAssertTrue(store.pending.isEmpty)
        XCTAssertEqual(FailingProtocol.calls, 1)
        do { let _: OK = try await api.post("/api/send", body: message.ref.payload); XCTFail("Must fail") } catch { }
        XCTAssertEqual(FailingProtocol.calls, 2, "Sending must never retry itself")
    }
    func testNativeTimeoutsOutlastTheMini() {
        XCTAssertGreaterThan(APIClient.timeout(for: "/api/send"), 120)
        XCTAssertGreaterThan(APIClient.timeout(for: "/api/draft-preview"), 120)
        XCTAssertGreaterThan(APIClient.timeout(for: "/api/action"), 90)
        XCTAssertGreaterThan(APIClient.timeout(for: "/api/chat/events"), 25)
    }
    func testOnlyAuthoritativeDraftGoneResponseIsTypedWithoutRetry() async {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [FailingProtocol.self]
        let client = APIClient(baseURL: URL(string: "https://mini.example.ts.net"), session: URLSession(configuration: config))
        defer { FailingProtocol.responseCode = 502; FailingProtocol.responseBody = #"{"error":"The mini is unavailable"}"# }
        for (status, code, missing) in [(410, "draft_gone", true), (502, "draft_gone", false), (410, "other", false)] {
            FailingProtocol.calls = 0; FailingProtocol.responseCode = status
            FailingProtocol.responseBody = "{\"error\":\"Unavailable\",\"code\":\"\(code)\"}"
            do { let _: DraftPreview = try await client.get("/api/draft-preview"); XCTFail("Must fail") }
            catch MailError.draftUnavailable { XCTAssertTrue(missing) }
            catch { XCTAssertFalse(missing) }
            XCTAssertEqual(FailingProtocol.calls, 1)
        }
    }
    func testMissingDraftClearsAllCachedReferencesAndPreservesLocalReply() async throws {
        Fixtures.missingDrafts = [20]
        defer { Fixtures.missingDrafts = [] }
        let store = MailStore(api: api(), cache: cache()); store.tab = .drafts
        var old = try APIClient.decoder.decode(MailMessage.self, from: JSONSerialization.data(withJSONObject: Fixtures.message()))
        old.folder = "Drafts"; old.uid = 20
        var other = old; other.uid = 21
        store.rows = [old, other]; store.selected = old.ref; store.review = old.ref
        store.snapshot.lists = [store.listKey: [old, other], "saved-search": [old, other]]
        store.snapshot.messages[old.id] = old; store.snapshot.html[old.id] = "Old draft body"
        let reply = LocalDraft(account: "work", to: "jordan@example.org", body: "Keep my unsent typing")
        store.remember(reply)
        await store.refreshAfterMissingDraft(old.ref)
        XCTAssertFalse(store.snapshot.lists.values.flatMap { $0 }.contains { $0.ref == old.ref })
        XCTAssertEqual(store.snapshot.lists["saved-search"], [other])
        XCTAssertNil(store.snapshot.messages[old.id]); XCTAssertNil(store.snapshot.html[old.id])
        XCTAssertNil(store.selected); XCTAssertNil(store.review)
        XCTAssertEqual(store.recoveredDraft, reply)
        XCTAssertEqual(MailStore(api: api(), cache: store.cache).recoveredDraft, reply)
    }
    func testAccountFolderUIDIdentityDoesNotCollide() {
        XCTAssertNotEqual(MessageRef(account: "ab", folder: "c", uid: 1).id, MessageRef(account: "a", folder: "bc", uid: 1).id)
        XCTAssertNotEqual(MessageRef(account: "a", folder: "Inbox", uid: 1).id, MessageRef(account: "a", folder: "Archive", uid: 1).id)
    }
    func testDraftSurvivesRelaunchAndCacheClearing() throws {
        let disk = cache(); defer { try? FileManager.default.removeItem(at: disk.directory) }
        let store = MailStore(api: api(), cache: disk)
        let draft = LocalDraft(account: "work", to: "jordan@example.org", body: "An unfinished reply")
        store.remember(draft); store.clearCache()
        let relaunched = MailStore(api: api(), cache: disk)
        XCTAssertEqual(relaunched.recoveredDraft, draft)
        XCTAssertEqual(relaunched.snapshot.messages.count, 0)
        relaunched.clearDraft()
        XCTAssertNil(MailStore(api: api(), cache: disk).recoveredDraft)
    }
    func testReadFlagPreservesOfflineBodyAndInvalidatesOtherLists() async throws {
        Fixtures.read = []; Fixtures.archived = []
        let store = MailStore(api: api(), cache: cache())
        await store.load()
        let ref = store.rows[0].ref
        let message = try await store.message(ref)
        let html = try await store.body(ref)
        store.snapshot.lists["old-search"] = [message]
        let success = await store.action("read", message)
        XCTAssertTrue(success)
        XCTAssertTrue(store.snapshot.messages[ref.id]?.read == true)
        XCTAssertEqual(store.snapshot.html[ref.id], html)
        XCTAssertNil(store.snapshot.lists["old-search"])
        let relaunched = MailStore(api: api(), cache: store.cache)
        XCTAssertEqual(relaunched.snapshot.html[ref.id], html)
    }
    func testPreviewDecodesAllRecipientsAndDigest() async throws {
        let preview: DraftPreview = try await api().get("/api/draft-preview")
        XCTAssertEqual(preview.bcc.first?.email, "copy@example.org")
        XCTAssertEqual(preview.from.email, "me@example.com")
        XCTAssertEqual(preview.digest.count, 64)
        XCTAssertEqual(preview.ref.account, "work")
    }
    func testStreamingReconciliationAndPermissionExpiry() {
        let chat = ChatStore(api: api())
        chat.apply(["type": "text_delta", "text": "Hel"])
        chat.apply(["type": "text_delta", "text": "lo"])
        chat.apply(["type": "assistant_text", "text": "Hello there"])
        XCTAssertEqual(chat.items.count, 1); XCTAssertEqual(chat.items[0].text, "Hello there")
        chat.apply(["type": "permission_request", "request_id": "r1", "tool_name": "compose_draft", "rememberable": false, "input": ["to": "jordan@example.org"]])
        XCTAssertFalse(chat.items[1].rememberable)
        chat.apply(["type": "exited", "code": 1])
        XCTAssertEqual(chat.items[1].permissionState, "No longer waiting")
        XCTAssertFalse(chat.busy); XCTAssertFalse(chat.alive)
    }
    func testChatMarkdownKeepsBlocksAndUnfinishedStreamingCode() {
        let blocks = ChatMarkdownBlock.parse("## Next steps\n\nA **short** reply.\n\n- Tuesday\n1. Review\n> Quoted\n\n```swift\nlet day = 2")
        XCTAssertEqual(blocks, [.heading("Next steps"), .paragraph("A **short** reply."),
            .list("•", "Tuesday"), .list("1.", "Review"), .quote("Quoted"), .code("swift", "let day = 2")])
    }
    func testChatSendKeepsUserBeforeStreamAndPreservesEditedDraft() async {
        let chat = ChatStore(api: api())
        await chat.start(seed: nil)
        chat.draft = "Find my mail"
        let sent = await chat.send(chat.draft)
        XCTAssertTrue(sent)
        XCTAssertEqual(chat.items.first?.kind, .user)
        XCTAssertEqual(chat.draft, "")
        chat.draft = "A follow-up in progress"
        chat.apply(["type": "text_delta", "text": "Found it."])
        chat.apply(["type": "turn_done"])
        XCTAssertEqual(chat.draft, "A follow-up in progress")
        XCTAssertFalse(chat.busy)
        chat.pause()
    }
    func testChatContextReachesTransportWithoutResettingHistory() async {
        let chat = ChatStore(api: api())
        let original = MessageRef(account: "work", folder: "INBOX", uid: 10)
        let other = MessageRef(account: "personal", folder: "Receipts", uid: 42)
        await chat.start(seed: original)
        let starts = Fixtures.chatStarts
        let sent = await chat.send("Summarize this", context: other)
        XCTAssertTrue(sent)
        XCTAssertTrue(Fixtures.lastChatText.contains("\"account_id\":\"personal\""))
        XCTAssertTrue(Fixtures.lastChatText.contains("\"uid\":42"))
        XCTAssertEqual(chat.items.first?.text, "Summarize this", "Transport instructions do not appear in the user's bubble")
        chat.draft = "Unsent question"
        await chat.ensure(seed: other)
        XCTAssertEqual(Fixtures.chatStarts, starts, "Changing context or presentation must not restart the provider")
        XCTAssertEqual(chat.draft, "Unsent question")
        XCTAssertEqual(chat.items.first?.text, "Summarize this")
        XCTAssertTrue(ChatStore.requestText("Find receipts", context: nil).contains("all of my mail"))
        chat.pause()
    }
    func testCurrentEmailContextFollowsSelectionWithoutFallingBack() {
        let store = MailStore(api: api(), cache: cache())
        store.chatContextScope = .currentEmail
        store.selected = .init(account: "work", folder: "INBOX", uid: 10)
        XCTAssertEqual(store.chatSeed?.uid, 10)
        store.selected = .init(account: "work", folder: "INBOX", uid: 11)
        XCTAssertEqual(store.chatSeed?.uid, 11)
        store.selected = nil
        XCTAssertNil(store.chatSeed)
        XCTAssertEqual(store.chatContextScope, .currentEmail, "Opening the inbox must not silently select mailbox-wide context")
    }
}
