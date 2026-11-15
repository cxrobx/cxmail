import XCTest

final class CXMailUITests: XCTestCase {
    var app: XCUIApplication!
    override func setUpWithError() throws { continueAfterFailure = false }
    func launch(_ extra: [String] = [], reset: Bool = true) {
        app = XCUIApplication()
        app.launchArguments = ["--uitesting"] + (reset ? ["--reset"] : []) + extra
        app.launch()
    }
    func capture(_ name: String) {
        let shot = XCTAttachment(screenshot: app.screenshot()); shot.name = name; shot.lifetime = .keepAlways; add(shot)
    }
    func openMessage() {
        let message = app.buttons["message-10"]
        XCTAssertTrue(message.waitForExistence(timeout: 10)); message.tap()
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15), "Sanitized HTML must actually render")
    }
    func confirmSend() {
        let query = app.buttons.matching(identifier: "confirmSend")
        XCTAssertTrue(query.element(boundBy: 0).waitForExistence(timeout: 5))
        let matches = query.allElementsBoundByIndex
        // iOS 27 exposes a confirmation action and its nested button with the
        // same AX id. Accept that exact duplicate, never a different target.
        XCTAssertTrue((1...2).contains(matches.count))
        if matches.count == 2 { XCTAssertEqual(matches[0].frame, matches[1].frame) }
        matches[matches.count - 1].tap()
    }
    func tapDraftsTab() {
        let query = app.buttons.matching(identifier: "Drafts")
        XCTAssertTrue(query.element(boundBy: 0).waitForExistence(timeout: 5))
        if query.count == 2 {
            // iPad's floating tab exposes a nested button inside its cell.
            // Select that specific descendant, never an unrelated duplicate.
            let nested = query.element(boundBy: 0).buttons.matching(identifier: "Drafts")
            XCTAssertEqual(nested.count, 1); nested.element.tap()
        } else {
            XCTAssertEqual(query.count, 1); query.element.tap()
        }
    }
    func testInboxReaderReplyRecovery() {
        launch(); capture("Inbox")
        openMessage(); capture("Reader")
        app.buttons["readerReply"].tap()
        let body = app.textFields["composeBody"]
        XCTAssertTrue(body.waitForExistence(timeout: 5))
        body.tap(); body.typeText("Tuesday works well. See you then!")
        capture("Reply with keyboard")
        app.buttons["cancelCompose"].tap()
        app.buttons["Keep on this iPhone"].tap()
        app.terminate()
        launch(reset: false)
        let recover = app.buttons["recoverDraft"]
        XCTAssertTrue(recover.waitForExistence(timeout: 10)); recover.tap()
        XCTAssertEqual(app.textFields["composeBody"].value as? String, "Tuesday works well. See you then!")
        capture("Recovered reply")
    }
    func testDenseInboxAndSafeGlassActions() {
        launch()
        let emptyPreview = app.buttons["message-12"]
        XCTAssertTrue(emptyPreview.waitForExistence(timeout: 10))
        XCTAssertLessThanOrEqual(emptyPreview.frame.height, 84, "An empty snippet must not leave a large empty card")
        XCTAssertLessThanOrEqual(app.buttons["message-10"].frame.height, 104)
        XCTAssertTrue(app.buttons["message-13"].isHittable, "At least four realistic messages must fit on the closed Duo")
        XCTAssertLessThanOrEqual(app.buttons["compose"].frame.height, 52)
        XCTAssertLessThanOrEqual(app.buttons["openChat"].frame.width, 52, "Chat is its own compact icon button")
        XCTAssertLessThanOrEqual(app.buttons["compose"].frame.width, 52)
        XCTAssertTrue(app.buttons["All"].exists)
        XCTAssertLessThan(app.buttons["All"].frame.minX, app.buttons["Primary"].frame.minX)
        XCTAssertFalse(app.buttons["All mail"].exists)
        capture("Dense glass inbox")
    }
    func testBodyFailureIsRetryableAndNotMetadata() {
        launch(["--body-unavailable"])
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout: 10)); app.buttons["message-10"].tap()
        let retry = app.buttons["retryBody"]
        XCTAssertTrue(retry.waitForExistence(timeout: 10))
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS 'Message-ID:'")).element.exists)
        capture("Body loading failure")
        retry.tap()
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
    }
    func testComposeReviewAndExplicitSend() {
        launch()
        XCTAssertTrue(app.buttons["compose"].waitForExistence(timeout: 10)); app.buttons["compose"].tap()
        let to = app.textFields["composeTo"]; XCTAssertTrue(to.waitForExistence(timeout: 5)); to.tap(); to.typeText("jordan@example.org")
        app.textFields["composeSubject"].tap(); app.textFields["composeSubject"].typeText("Tuesday")
        app.textFields["composeBody"].tap(); app.textFields["composeBody"].typeText("Tuesday works well.")
        app.buttons["reviewCompose"].tap()
        let send = app.buttons["sendReviewedDraft"]
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["copy@example.org"].exists)
        capture("All recipients review")
        send.tap()
        XCTAssertTrue(app.buttons["Send now"].waitForExistence(timeout: 5))
        confirmSend()
        XCTAssertTrue(app.alerts.staticTexts["Sent."].waitForExistence(timeout: 10))
    }
    func testCompactSendToolbarScheduleAndCancel() {
        launch()
        if app.frame.width >= 669 {
            app.terminate(); XCUIDevice.shared.orientation = .landscapeLeft; launch(["--duo-flat"])
        }
        tapDraftsTab()
        XCTAssertTrue(app.buttons["message-20"].waitForExistence(timeout: 10)); app.buttons["message-20"].tap()
        let send = app.buttons["sendReviewedDraft"]
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertLessThanOrEqual(send.frame.width, 52, "The send action must stay an icon-sized control")
        XCTAssertLessThanOrEqual(send.frame.height, 52)
        XCTAssertGreaterThanOrEqual(send.frame.height, 44)
        let options = app.buttons["sendOptions"]
        XCTAssertLessThanOrEqual(options.frame.maxX - send.frame.minX, 112, "No full-width send banner")
        capture("Compact glass send toolbar")
        options.tap()
        let scheduleOption = app.buttons["Schedule send…"]
        XCTAssertTrue(scheduleOption.waitForExistence(timeout: 5)); capture("Send now and schedule options")
        scheduleOption.tap()
        XCTAssertTrue(app.buttons["scheduleDraft"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.staticTexts["copy@example.org"].exists, "Scheduling also shows Bcc recipients")
        app.buttons["In one hour"].tap(); capture("Scheduled send date and recipient review")
        app.buttons["scheduleDraft"].tap()
        let confirm = app.buttons.matching(identifier: "confirmSchedule").firstMatch
        XCTAssertTrue(confirm.waitForExistence(timeout: 5)); confirm.tap()
        XCTAssertTrue(app.buttons["reviewSchedule"].waitForExistence(timeout: 10))
        XCTAssertFalse(send.isEnabled, "An already queued draft cannot be sent twice from review")
        capture("Draft scheduled on mini")
        app.buttons["reviewSchedule"].tap()
        let cancel = app.buttons["cancelSchedule-1"]
        XCTAssertTrue(cancel.waitForExistence(timeout: 5)); capture("Manage scheduled sends"); cancel.tap()
        let confirmCancel = app.buttons.matching(identifier: "confirmCancelSchedule").firstMatch
        XCTAssertTrue(confirmCancel.waitForExistence(timeout: 5)); confirmCancel.tap()
        XCTAssertTrue(app.staticTexts["Cancelled"].waitForExistence(timeout: 5))
        capture("Cancelled schedule keeps draft")
        app.buttons["closeScheduled"].tap()
        let enabled = XCTNSPredicateExpectation(predicate: NSPredicate(format: "enabled == true"), object: send)
        XCTAssertEqual(XCTWaiter.wait(for: [enabled], timeout: 5), .completed)
    }
    func testStaleDraftCannotSendOrBlindlyRetry() {
        launch(["--stale-draft"])
        app.buttons["Drafts"].tap()
        XCTAssertTrue(app.buttons["message-20"].waitForExistence(timeout: 10)); app.buttons["message-20"].tap()
        let send = app.buttons["sendReviewedDraft"]; XCTAssertTrue(send.waitForExistence(timeout: 10)); send.tap()
        confirmSend()
        XCTAssertTrue(app.staticTexts["sendError"].waitForExistence(timeout: 10))
        XCTAssertFalse(send.isEnabled)
        capture("Changed draft refused")
    }
    func testMissingDraftRefreshesListAndOpensCurrentDraft() {
        launch(["--missing-draft"])
        if app.frame.width >= 669 {
            app.terminate(); XCUIDevice.shared.orientation = .landscapeLeft; launch(["--duo-flat", "--missing-draft"])
        }
        tapDraftsTab()
        let stale = app.buttons["message-20"]
        XCTAssertTrue(stale.waitForExistence(timeout: 10)); stale.tap()
        let explanation = app.descendants(matching: .any)["draftUpdated"]
        XCTAssertTrue(explanation.waitForExistence(timeout: 10)); capture("Missing draft clears and refreshes")
        XCTAssertFalse(stale.exists, "An obsolete revision must not remain a tappable ghost")
        let current = app.buttons["message-21"]
        XCTAssertTrue(current.waitForExistence(timeout: 10)); current.tap()
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["sendReviewedDraft"].isEnabled)
        capture("Current draft opens after stale revision")
    }
    func testDraftConnectionFailureKeepsDraftAndCanRetryReview() {
        launch(["--draft-preview-unavailable"])
        tapDraftsTab()
        XCTAssertTrue(app.buttons["message-20"].waitForExistence(timeout: 10)); app.buttons["message-20"].tap()
        XCTAssertTrue(app.staticTexts["sendError"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["sendReviewedDraft"].isEnabled)
        capture("Connection failure keeps draft retryable")
        app.buttons["Refresh review"].tap()
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["sendReviewedDraft"].isEnabled)
    }
    func testClaudePermissionAndStopControls() {
        launch()
        XCTAssertTrue(app.buttons["openChat"].waitForExistence(timeout: 10)); app.buttons["openChat"].tap()
        let input = app.textFields["chatInput"]; XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Draft a reply")
        app.buttons["chatSend"].tap()
        let deny = app.buttons["permissionDeny"]
        XCTAssertTrue(deny.waitForExistence(timeout: 10))
        capture("Claude permission")
        deny.tap(); XCTAssertTrue(app.staticTexts["Denied"].waitForExistence(timeout: 5))
        XCTAssertEqual(app.buttons["chatSend"].label, "Stop Claude")
        app.buttons["chatSend"].tap()
        let stopped = XCTNSPredicateExpectation(predicate: NSPredicate(format: "label == 'Send message'"), object: app.buttons["chatSend"])
        XCTAssertEqual(XCTWaiter.wait(for: [stopped], timeout: 5), .completed)
    }
    func testChatMarkdownAndComposer() {
        launch(["--chat-rich"])
        app.buttons["openChat"].tap()
        let input = app.textFields["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Write a short plan")
        capture("Chat multiline composer")
        app.buttons["chatSend"].tap()
        XCTAssertTrue(app.staticTexts["Next steps"].waitForExistence(timeout: 10))
        app.scrollViews.containing(.staticText, identifier: "Next steps").firstMatch.swipeDown()
        XCTAssertTrue(app.staticTexts["Reply to Jordan"].exists)
        XCTAssertTrue(app.staticTexts["Tuesday works well."].exists)
        XCTAssertFalse(app.staticTexts["## Next steps"].exists)
        XCTAssertEqual(app.buttons["chatSend"].label, "Send message")
        capture("Formatted chat transcript")
        app.buttons["chatContext"].tap()
        XCTAssertFalse(app.buttons["Current email"].isEnabled, "Email context requires an open message")
        app.buttons["Your mail"].tap()
    }
    func testChatContextPickerKeepsConversationAndDraft() {
        launch(["--show-reader", "--chat-rich"])
        XCTAssertTrue(app.buttons["readerChat"].waitForExistence(timeout: 10)); app.buttons["readerChat"].tap()
        let input = app.textFields["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Write a short plan"); app.buttons["chatSend"].tap()
        XCTAssertTrue(app.staticTexts["Next steps"].waitForExistence(timeout: 10))
        input.tap(); input.typeText("Keep this draft")
        let context = app.buttons["chatContext"]
        XCTAssertEqual(context.value as? String, "Current email")
        context.tap(); capture("Chat context choices"); app.buttons["Your mail"].tap()
        XCTAssertEqual(context.value as? String, "Your mail")
        XCTAssertEqual(input.value as? String, "Keep this draft")
        XCTAssertTrue(app.staticTexts["Next steps"].exists)
        context.tap(); app.buttons["Current email"].tap()
        XCTAssertEqual(context.value as? String, "Current email")
        XCTAssertEqual(input.value as? String, "Keep this draft")
        XCTAssertTrue(app.staticTexts["Next steps"].exists)
        capture("Selectable chat context")
    }
    func testDockChatResizeAndPopOut() throws {
        XCUIDevice.shared.orientation = .landscapeLeft
        launch()
        guard app.frame.width > 867 else { throw XCTSkip("Run docked chat on the open-width profile.") }
        app.terminate(); launch(["--duo-flat", "--show-reader", "--chat-rich"])
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        app.buttons["readerChat"].tap()
        let input = app.textFields["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Write a short plan")
        app.buttons["chatSend"].tap()
        XCTAssertTrue(app.staticTexts["Next steps"].waitForExistence(timeout: 10))
        input.tap(); input.typeText("Keep this draft while docking.")
        XCTAssertTrue(app.buttons["chatPresentation"].isEnabled); app.buttons["chatPresentation"].tap()
        let divider = app.descendants(matching: .any)["chatDivider"]
        XCTAssertTrue(divider.waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.staticTexts["Next steps"].exists, "Docking must retain the conversation")
        XCTAssertEqual(input.value as? String, "Keep this draft while docking.")
        let before = app.webViews.firstMatch.frame.width
        capture("Email and chat split")
        divider.press(forDuration: 0.2, thenDragTo: input)
        XCTAssertGreaterThan(app.webViews.firstMatch.frame.width, before + 30)
        capture("Resized chat split")
        input.tap(); input.typeText(" Still docked.")
        XCTAssertTrue(divider.exists, "Showing the keyboard must keep split chat docked")
        let edited = input.value as? String ?? ""
        XCTAssertTrue(edited.contains("Keep this draft while docking.")); XCTAssertTrue(edited.contains("Still docked."))
        // The iPad keyboard pans the fixed-size virtual phone. Restore its
        // viewport before testing navigation controls; real Duo windows resize.
        app.keyboards.buttons["Hide keyboard"].tap()
        app.buttons["chatPresentation"].tap()
        XCTAssertTrue(app.buttons["chatPresentation"].waitForExistence(timeout: 5))
        XCTAssertEqual(input.value as? String, edited)
        XCTAssertFalse(divider.exists)
        app.buttons["closeChat"].tap()
        XCTAssertTrue(app.buttons["readerReply"].isHittable)
        XCTAssertGreaterThan(app.webViews.firstMatch.frame.width, 500, "Undocking restores the original mail reader")
        capture("Mail toolbar restored")
    }
    func testSearchAndAccountSelection() {
        launch()
        app.buttons["Search"].tap()
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5)); search.tap(); search.typeText("project")
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout: 10))
        capture("Search")
    }
    func testOfflineAndEmptyStates() {
        launch(["--offline"])
        XCTAssertTrue(app.buttons["retryConnection"].waitForExistence(timeout: 10)); capture("Offline")
        app.terminate(); launch(["--empty"])
        XCTAssertTrue(app.staticTexts["You’re all caught up"].waitForExistence(timeout: 10)); capture("Empty inbox")
    }
    func testOpenWideReaderKeepsInboxAndRespectsSafeArea() {
        XCUIDevice.shared.orientation = .landscapeLeft
        launch()
        if app.frame.width > 760 { app.terminate(); launch(["--duo-wide"]) }
        openMessage()
        let initialHierarchy = XCTAttachment(string: app.debugDescription); initialHierarchy.name = "Split geometry"; initialHierarchy.lifetime = .keepAlways; add(initialHierarchy)
        let pane = app.otherElements["detailPane"]
        if app.frame.width > 760 {
            XCTAssertTrue(pane.exists)
            XCTAssertTrue(app.buttons["message-11"].isHittable)
            XCTAssertTrue(app.buttons["readerReply"].isHittable)
            XCTAssertLessThan(app.buttons["readerReply"].frame.maxX, app.frame.maxX)
        }
        capture("Wide inbox and reader")
        XCUIDevice.shared.orientation = .portrait
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 5))
        capture("Tall reader keeps selection")
    }
    func testExactOpenDuoViewport() throws {
        XCUIDevice.shared.orientation = .landscapeLeft
        launch()
        guard app.frame.width > 867 else { throw XCTSkip("Run the exact open-width profile on the iPad simulator.") }
        app.terminate(); launch(["--duo-wide"])
        // This test runs on an iPad simulator so the exact 867x635 usable
        // viewport fits. The fixture supplies the measured 40pt fold.
        XCTAssertGreaterThan(app.frame.width, 867)
        openMessage()
        let splitHierarchy = XCTAttachment(string: app.debugDescription); splitHierarchy.name = "Native split geometry"; splitHierarchy.lifetime = .keepAlways; add(splitHierarchy)
        XCTAssertTrue(app.otherElements["detailPane"].exists)
        XCTAssertTrue(app.buttons["message-11"].isHittable)
        XCTAssertTrue(app.buttons["readerReply"].isHittable)
        XCTAssertTrue(app.buttons["readerArchive"].isHittable)
        XCTAssertEqual(app.buttons["message-11"].frame.width, 455.5, accuracy: 1, "The active fold determines the inbox width")
        XCTAssertEqual(app.webViews.firstMatch.frame.minX - app.buttons["message-11"].frame.maxX, 52, accuracy: 2, "Keep the measured 40pt division plus the 12pt email margin free")
        XCTAssertLessThanOrEqual(app.buttons["readerReply"].frame.maxX, app.frame.maxX)
        XCTAssertGreaterThan(app.buttons["readerReply"].frame.minX, app.buttons["message-11"].frame.maxX + 39)
        let hierarchy = XCTAttachment(string: app.debugDescription); hierarchy.name = "Open viewport hierarchy"; hierarchy.lifetime = .keepAlways; add(hierarchy)
        capture("Exact open Duo viewport")
        app.buttons["readerReply"].tap()
        let body = app.textFields["composeBody"]
        XCTAssertTrue(body.waitForExistence(timeout: 5)); body.tap(); body.typeText("Keep this through a fold.")
        XCUIDevice.shared.orientation = .portrait
        XCTAssertEqual(body.value as? String, "Keep this through a fold.")
        capture("Open composer retains reply on rotation")
        app.terminate()
        XCUIDevice.shared.orientation = .landscapeLeft
        launch(["--duo-flat", "--show-reader"])
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertGreaterThan(app.webViews.firstMatch.frame.width, 500, "A flat Duo should give the reader most of the usable width")
        let inbox = app.buttons["message-11"].frame
        let detail = app.webViews.firstMatch.frame
        let flatHierarchy = XCTAttachment(string: app.debugDescription); flatHierarchy.name = "Flat split geometry"; flatHierarchy.lifetime = .keepAlways; add(flatHierarchy)
        XCTAssertGreaterThanOrEqual(detail.minX - inbox.maxX, 0, "Inbox content must not extend into the reader")
        XCTAssertLessThan(detail.minX - inbox.maxX, 24, "The status safe area must not leave a second blank strip inside the inbox")
        XCTAssertGreaterThan(inbox.width, 290, "The 329pt flat inbox must retain its usable width")
        capture("Flat Duo glass workspace")
    }
    func testSplitPaneStatusInset() {
        // Constrain the same split tree on the Duo runtime too. The closed
        // simulator clips this wide canvas; AX still reports pane geometry.
        launch(["--duo-flat", "--show-reader"])
        XCTAssertTrue(app.buttons["message-11"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        let hierarchy = XCTAttachment(string: app.debugDescription); hierarchy.name = "Status inset hierarchy"; hierarchy.lifetime = .keepAlways; add(hierarchy)
        let inbox = app.buttons["message-11"].frame
        let detail = app.webViews.firstMatch.frame
        XCTAssertGreaterThan(inbox.width, 290)
        XCTAssertGreaterThanOrEqual(detail.minX - inbox.maxX, 0)
        XCTAssertLessThan(detail.minX - inbox.maxX, 24)
    }
    func testDeviceLandscapeSidebar() throws {
        XCUIDevice.shared.orientation = .landscapeLeft
        launch(["--show-reader"])
        guard app.frame.width >= 760 else { throw XCTSkip("Requires the Duo's inner display or a wide device.") }
        XCTAssertTrue(app.buttons["message-11"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        let inbox = app.buttons["message-11"].frame
        let detail = app.webViews.firstMatch.frame
        let hierarchy = XCTAttachment(string: app.debugDescription); hierarchy.name = "Native landscape sidebar"; hierarchy.lifetime = .keepAlways; add(hierarchy)
        // Compare actual content boundaries, allowing the active fold and
        // the reader's paper margin. A duplicated status inset exceeds this.
        XCTAssertGreaterThanOrEqual(detail.minX - inbox.maxX, 0)
        XCTAssertLessThan(detail.minX - inbox.maxX, 64)
        XCTAssertGreaterThan(inbox.width, 290)
        XCTAssertLessThan(app.buttons["compose"].frame.maxX, detail.minX)
        capture("Native landscape sidebar")
    }
    func testPortraitInboxDrawer() throws {
        XCUIDevice.shared.orientation = .portrait
        launch()
        guard app.frame.width >= 669 else { throw XCTSkip("Requires a viewport large enough for open-tall Duo geometry.") }
        app.terminate(); launch(["--duo-tall"])
        let show = app.buttons["showInbox"]
        XCTAssertTrue(show.waitForExistence(timeout: 5)); XCTAssertTrue(show.isHittable)
        XCTAssertFalse(app.buttons["message-11"].isHittable, "Portrait must start with the inbox drawer closed")
        capture("Portrait starts with closed inbox")
        show.tap(); openMessage()
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        let detail = app.webViews.firstMatch.frame
        XCTAssertGreaterThan(detail.width, 600, "Portrait keeps the message at full width")
        let hierarchy = XCTAttachment(string: app.debugDescription); hierarchy.name = "Portrait toolbar geometry"; hierarchy.lifetime = .keepAlways; add(hierarchy)
        XCTAssertTrue(show.isHittable); show.tap()
        let row = app.buttons["message-11"]
        XCTAssertTrue(row.waitForExistence(timeout: 5)); XCTAssertTrue(row.isHittable)
        XCTAssertLessThan(row.frame.width, 421)
        XCTAssertGreaterThan(row.frame.width, 380, "The portrait drawer should retain its requested readable width")
        XCTAssertLessThan(row.frame.maxX, detail.maxX - 150, "Inbox should overlay only part of the message")
        let openHierarchy = XCTAttachment(string: app.debugDescription); openHierarchy.name = "Open portrait controls"; openHierarchy.lifetime = .keepAlways; add(openHierarchy)
        capture("Portrait inbox drawer")
        row.tap()
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 10))
        XCTAssertEqual(app.staticTexts["readerSubject"].label, "The next steps for our project")
        XCTAssertTrue(show.isHittable)
        show.tap()
        let hide = app.buttons["hideInbox"]
        let ready = XCTNSPredicateExpectation(predicate: NSPredicate(format: "hittable == true"), object: hide)
        XCTAssertEqual(XCTWaiter.wait(for: [ready], timeout: 5), .completed, "Wait for the native drawer transition before tapping its trailing control")
        hide.tap(); XCTAssertTrue(show.isHittable)
        capture("Portrait message after closing drawer")
    }
}
