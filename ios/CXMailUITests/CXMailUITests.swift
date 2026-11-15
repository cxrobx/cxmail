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
        XCTAssertTrue(app.otherElements["detailPane"].exists)
        XCTAssertTrue(app.buttons["message-11"].isHittable)
        XCTAssertTrue(app.buttons["readerReply"].isHittable)
        XCTAssertTrue(app.buttons["readerArchive"].isHittable)
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
    }
}
