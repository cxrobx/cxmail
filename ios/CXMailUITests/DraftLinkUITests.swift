import XCTest

final class DraftLinkUITests: XCTestCase {
    let id = "0278e6ec-4dd4-4661-bd7e-b1cf34edb637"
    func testColdLaunchDraftLinkShowsExactReviewAndHTMLWithoutSending() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--reset", "--draft-link", "cxmail://draft/\(id)"]
        app.launch()
        XCTAssertTrue(app.staticTexts["Bcc · hidden recipients"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["sendReviewedDraft"].exists)
        XCTAssertFalse(app.buttons["confirmSend"].exists, "Navigation never starts send confirmation")
        let shot = XCTAttachment(screenshot: app.screenshot()); shot.name = "Linked draft review"; shot.lifetime = .keepAlways; add(shot)
    }
    func testOperatingSystemDispatchesHTTPSLinkIntoDraftReview() throws {
        guard let host = Bundle(for: Self.self).object(forInfoDictionaryKey: "CXMailLinkHost") as? String else {
            XCTFail("The UI test bundle must receive CXMailLinkHost from project.yml")
            return
        }
        if host == "links.invalid" { throw XCTSkip("Configure the deployed association host before testing OS dispatch") }
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--reset"]
        app.launch()
        XCUIDevice.shared.press(.home)
        XCUIDevice.shared.system.open(URL(string: "https://\(host)/draft/\(id)")!)
        XCTAssertTrue(app.wait(for: .runningForeground, timeout: 20), "iOS must dispatch the universal link directly to CXMail")
        XCTAssertTrue(app.staticTexts["Bcc · hidden recipients"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertFalse(app.buttons["confirmSend"].exists)
    }
    func testOfflineLinkOffersRetryAndDismissal() {
        let app = XCUIApplication()
        app.launchArguments = ["--uitesting", "--reset", "--draft-link-unavailable", "--draft-link", "cxmail://draft/\(id)"]
        app.launch()
        XCTAssertTrue(app.buttons["retryDraftLink"].waitForExistence(timeout: 15))
        XCTAssertFalse(app.buttons["sendReviewedDraft"].exists)
        app.buttons["dismissDraftLink"].tap()
        XCTAssertFalse(app.buttons["retryDraftLink"].exists)
    }
}
