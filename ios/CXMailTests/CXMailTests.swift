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

final class ImageDocumentProtocol: URLProtocol {
    static var permission: String?
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        let headers = Self.permission.map { ["X-CXMail-Images-Allowed": $0] } ?? [:]
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: headers)!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data("<p>Fictional message</p>".utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    override func stopLoading() {}
}

final class PushUpdateProtocol: URLProtocol {
    static let lock = NSLock()
    static var registration: PushUpdateProtocol?
    static var started: XCTestExpectation?
    static var serverEnabled = false
    override class func canInit(with request: URLRequest) -> Bool { true }
    override class func canonicalRequest(for request: URLRequest) -> URLRequest { request }
    override func startLoading() {
        if request.url?.path == "/api/notifications/device" {
            Self.lock.lock(); Self.registration = self; let started = Self.started; Self.lock.unlock()
            started?.fulfill()
        } else {
            Self.lock.lock(); Self.serverEnabled = false; Self.lock.unlock()
            complete()
        }
    }
    override func stopLoading() {}
    func complete() {
        let response = HTTPURLResponse(url: request.url!, statusCode: 200, httpVersion: nil, headerFields: ["Content-Type":"application/json"])!
        client?.urlProtocol(self, didReceive: response, cacheStoragePolicy: .notAllowed)
        client?.urlProtocol(self, didLoad: Data(#"{"configured":true,"trusted_senders":[]}"#.utf8))
        client?.urlProtocolDidFinishLoading(self)
    }
    static func releaseRegistration() {
        lock.lock(); let current = registration; registration = nil
        if current != nil { serverEnabled = true }; lock.unlock()
        current?.complete()
    }
}

@MainActor
final class CXMailTests: XCTestCase {
    func api() -> APIClient { APIClient(baseURL: URL(string: "https://mini.example.ts.net:18706"), fixture: true) }
    func cache() -> MailCache { MailCache(directory: FileManager.default.temporaryDirectory.appendingPathComponent(UUID().uuidString)) }
    let linkID = "0278e6ec-4dd4-4661-bd7e-b1cf34edb637"
    func testNotificationOptOutWinsOverLateRegistration() async throws {
        let name = "cxmail-push-test-" + UUID().uuidString
        let defaults = try XCTUnwrap(UserDefaults(suiteName: name))
        defer { defaults.removePersistentDomain(forName: name); PushUpdateProtocol.releaseRegistration() }
        defaults.set(true, forKey: "pushEnabled")
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [PushUpdateProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://fixture.example.ts.net"), session: URLSession(configuration: config))
        let push = PushNotifications(api: api, defaults: defaults)
        let started = expectation(description: "Registration waiting on mini")
        PushUpdateProtocol.lock.lock(); PushUpdateProtocol.started = started; PushUpdateProtocol.serverEnabled = false; PushUpdateProtocol.lock.unlock()
        let registering = Task { await push.registered(Data(repeating: 0xab, count: 32)) }
        await fulfillment(of: [started], timeout: 5)
        let disabling = Task { await push.disable() }
        for _ in 0..<20 { if !push.enabled { break }; await Task.yield() }
        XCTAssertFalse(push.enabled)
        PushUpdateProtocol.releaseRegistration()
        await registering.value; await disabling.value
        XCTAssertFalse(PushUpdateProtocol.serverEnabled, "Late registration must not re-enable delivery after opt-out")
        XCTAssertEqual(push.status, "Notifications are off")
    }

    func testNotificationTapIsNavigationOnlyAndRequiresExactReference() {
        XCTAssertEqual(PushNotifications.messageReference(["account":"work", "folder":"INBOX", "uid":12]),
                       MessageRef(account:"work", folder:"INBOX", uid:12))
        for info: [AnyHashable: Any] in [["url":"https://evil.example"], ["account":"work","folder":"INBOX","uid":0],
                                       ["account":"","folder":"INBOX","uid":1], ["account":"work","folder":"INBOX","uid":"12"]] {
            XCTAssertNil(PushNotifications.messageReference(info))
        }
    }

    func testDraftLinksAcceptOnlyExactTrustedNavigation() {
        XCTAssertEqual(DraftLink.parse(URL(string: "cxmail://draft/\(linkID)")!), linkID)
        XCTAssertEqual(DraftLink.parse(URL(string: "https://links.example.com/draft/\(linkID)")!, host: "links.example.com"), linkID)
        for url in ["https://evil.example/draft/\(linkID)", "http://links.example.com/draft/\(linkID)",
                    "https://links.example.com:443/draft/\(linkID)", "https://user@links.example.com/draft/\(linkID)",
                    "cxmail://draft/\(linkID)?server=https://evil.example", "cxmail://draft/\(linkID)#send",
                    "cxmail://draft/\(linkID)/", "cxmail://draft/abc", "cxmail://draft/\(linkID.uppercased())",
                    "cxmail://draft/\(linkID)/1", "cxmail://draft/%30\(linkID.dropFirst())"] {
            XCTAssertNil(DraftLink.parse(URL(string: url)!, host: "links.example.com"), url)
        }
    }
    func testDraftLinkResolvesCurrentRevisionAndOnlyOpensReview() async {
        Fixtures.draftLinkUID = 21
        defer { Fixtures.draftLinkUID = 20 }
        let store = MailStore(api: api(), cache: cache())
        let beforeSent = Fixtures.sent
        store.receiveDraftLink(URL(string: "cxmail://draft/\(linkID)")!)
        await store.resolvePendingDraftLink()
        XCTAssertEqual(store.review, MessageRef(account: "work", folder: "Drafts", uid: 21))
        XCTAssertNil(store.pendingDraftLink)
        XCTAssertEqual(Fixtures.sent, beforeSent)
    }
    func testReceivingTheSameLinkStartsAFreshResolutionRequest() {
        let store = MailStore(api: api(), cache: cache())
        let url = URL(string: "cxmail://draft/\(linkID)")!
        store.receiveDraftLink(url)
        let request = store.draftLinkRequest
        store.receiveDraftLink(url)
        XCTAssertEqual(store.draftLinkRequest, request + 1)
        XCTAssertEqual(store.pendingDraftLink, linkID)
    }
    func testDraftLinkOfflineRetainsLinkAndRecoveryForRetry() async {
        let store = MailStore(api: api(), cache: cache())
        Fixtures.draftLinkUnavailable = true
        defer { Fixtures.draftLinkUnavailable = false }
        store.receiveDraftLink(URL(string: "cxmail://draft/\(linkID)")!)
        await store.resolvePendingDraftLink()
        XCTAssertEqual(store.pendingDraftLink, linkID); XCTAssertNotNil(store.draftLinkError)
        XCTAssertNil(store.review)
        Fixtures.draftLinkUnavailable = false
        await store.resolvePendingDraftLink()
        XCTAssertEqual(store.review?.uid, 20)
    }
    func testSlowDraftLinkCannotReplaceNewerSelectionOrDismissal() async throws {
        Fixtures.draftLinkDelay = .milliseconds(100)
        defer { Fixtures.draftLinkDelay = .zero }
        let store = MailStore(api: api(), cache: cache())
        store.receiveDraftLink(URL(string: "cxmail://draft/\(linkID)")!)
        let work = Task { await store.resolvePendingDraftLink() }
        try await Task.sleep(for: .milliseconds(20))
        store.selected = .init(account: "work", folder: "INBOX", uid: 10)
        await work.value
        XCTAssertNil(store.review); XCTAssertEqual(store.selected?.uid, 10)
        let retry = Task { await store.resolvePendingDraftLink() }
        try await Task.sleep(for: .milliseconds(20)); store.dismissDraftLink()
        await retry.value
        XCTAssertNil(store.review); XCTAssertNil(store.pendingDraftLink)
    }
    func testDraftLinkNeverReplacesAnOpenComposer() async {
        let store = MailStore(api: api(), cache: cache())
        store.newDraft()
        let original = store.composer
        store.receiveDraftLink(URL(string: "cxmail://draft/\(linkID)")!)
        await store.resolvePendingDraftLink()
        XCTAssertEqual(store.composer?.id, original?.id)
        XCTAssertNotNil(store.pendingDraftLink); XCTAssertNil(store.review)
    }
    func testHybridSearchUsesRankedResultsAndStatus() async throws {
        let store=MailStore(api:api(),cache:cache())
        store.tab = .search; store.query = "project"
        await store.load()
        XCTAssertEqual(store.rows.map(\.uid),[10,11])
        XCTAssertEqual(store.searchStatus,"ready")
        XCTAssertFalse(store.hasMore)
        store.query = "no match"
        await store.load()
        XCTAssertTrue(store.rows.isEmpty)
        XCTAssertEqual(store.searchStatus,"ready")
    }
    func testChangedQueryInvalidatesInFlightSemanticResponseBeforeNextLoad() async throws {
        Fixtures.searchDelay = .milliseconds(100)
        defer { Fixtures.searchDelay = .zero }
        let store=MailStore(api:api(),cache:cache())
        store.tab = .search; store.query = "project"
        let oldKey=store.listKey
        let load=Task { await store.load() }
        try await Task.sleep(for: .milliseconds(30))
        store.query = "no match"
        await load.value
        XCTAssertNil(store.searchStatus, "Old query cannot install its semantic status or cursor during debounce")
        XCTAssertNil(store.snapshot.lists[oldKey], "Old response must not be committed as current search state")
        await store.load()
        XCTAssertTrue(store.rows.isEmpty)
        XCTAssertEqual(store.searchStatus,"ready")
    }
    func testExpiredSearchRefreshesFirstPageAndStaysOnline() async throws {
        Fixtures.expireSearchCursor = true
        defer { Fixtures.expireSearchCursor = false }
        let store=MailStore(api:api(),cache:cache())
        store.tab = .search; store.query = "project"
        await store.load()
        XCTAssertTrue(store.hasMore)
        await store.load(more:true)
        XCTAssertFalse(Fixtures.expireSearchCursor)
        XCTAssertFalse(store.hasMore)
        XCTAssertEqual(store.rows.map(\.uid),[10,11])
        XCTAssertTrue(store.online); XCTAssertNil(store.error)
    }
    func testBusySemanticSearchRetriesOnce() async throws {
        Fixtures.searchBusyCount = 1
        defer { Fixtures.searchBusyCount = 0 }
        let store=MailStore(api:api(),cache:cache())
        store.tab = .search; store.query = "project"
        await store.load()
        XCTAssertEqual(store.searchStatus,"ready")
        XCTAssertTrue(store.online)
    }
    func testTypingRetainsResultsWhileNextQueryIsLoading() async throws {
        let store=MailStore(api:api(),cache:cache())
        store.tab = .search; store.query = "project"
        await store.load()
        let previous=store.rows
        Fixtures.searchDelay = .milliseconds(100)
        defer { Fixtures.searchDelay = .zero }
        store.query = "no match"
        let load=Task { await store.load() }
        try await Task.sleep(for: .milliseconds(30))
        XCTAssertEqual(store.rows,previous,"Typing must not blank to skeletons on every query")
        await load.value
        XCTAssertTrue(store.rows.isEmpty)
    }
    func testAutomaticImagesRequireTrustAuthenticationAndMatchingSender() throws {
        let json = #"{"account":"work","folder":"INBOX","uid":10,"subject":"Notes","from_email":"jordan@example.org","date":"2026-11-15T12:00:00Z","snippet":"Hi","read":true,"starred":false,"attachments":false,"image_policy":{"authentication":{"status":"passed","domain":"example.org","provider":"Google","explanation":"Domain authenticated, not content verified."},"sender_email":"jordan@example.org","sender_trusted":true,"auto_load":true,"suspicious":false,"has_remote_images":true}}"#
        let message = try APIClient.decoder.decode(MailMessage.self, from: Data(json.utf8))
        XCTAssertTrue(message.permitsAutomaticImages)
        for status in ["failed", "unknown"] {
            var changed = message; changed.imagePolicy?.authentication.status = status
            XCTAssertFalse(changed.permitsAutomaticImages)
        }
        var changed = message; changed.imagePolicy?.senderTrusted = false
        XCTAssertFalse(changed.permitsAutomaticImages)
        changed = message; changed.imagePolicy?.suspicious = true
        XCTAssertFalse(changed.permitsAutomaticImages)
        changed = message; changed.imagePolicy?.senderEmail = "other@example.org"
        XCTAssertFalse(changed.permitsAutomaticImages)
        changed = message; changed.imagePolicy = nil; changed.imagesTrusted = true
        XCTAssertFalse(changed.permitsAutomaticImages, "The old address-only flag is never permission")
    }
    func testImageOptInsNeverPersistAnUnblockedOfflineBody() async throws {
        let store = MailStore(api: api(), cache: cache())
        let ref = MessageRef(account: "work", folder: "INBOX", uid: 10)
        _ = try await store.body(ref, images: true, automatic: true)
        XCTAssertNil(store.snapshot.html[ref.id])
        _ = try await store.body(ref)
        XCTAssertNotNil(store.snapshot.html[ref.id])
    }
    func testFinalBodyPermissionOverridesAnEarlierAutomaticDecision() async throws {
        let config = URLSessionConfiguration.ephemeral; config.protocolClasses = [ImageDocumentProtocol.self]
        let api = APIClient(baseURL: URL(string: "https://mini.example.ts.net:18706"), session: URLSession(configuration: config))
        let query = [URLQueryItem(name: "images", value: "auto")]
        for header in [nil, "0", "unexpected"] as [String?] {
            ImageDocumentProtocol.permission = header
            let document = try await api.imageDocument(query: query)
            XCTAssertFalse(document.imagesAllowed, "Missing or revoked final permission cannot enable WKWebView images")
        }
        ImageDocumentProtocol.permission = "1"
        let document = try await api.imageDocument(query: query)
        XCTAssertTrue(document.imagesAllowed)
        XCTAssertEqual(document.html, "<p>Fictional message</p>")
        ImageDocumentProtocol.permission = nil
    }
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
