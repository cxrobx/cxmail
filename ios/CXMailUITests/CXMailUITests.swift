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
        let shot = XCTAttachment(screenshot: XCUIScreen.main.screenshot()); shot.name = name; shot.lifetime = .keepAlways; add(shot)
    }
    func openMessage() {
        let message = app.buttons["message-10"]
        XCTAssertTrue(message.waitForExistence(timeout: 10)); message.tap()
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15), "Sanitized HTML must actually render")
    }
    func testInboxContextMenuOpensT3WithOptionalMessage() {
        XCUIDevice.shared.orientation = .portrait
        launch(["--session-fixture"])
        let row = app.buttons["message-10"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.65)).press(forDuration: 1)
        let action = app.buttons["Open in Session"].firstMatch
        XCTAssertTrue(action.waitForExistence(timeout: 5))
        action.tap()
        let dialog = app.alerts["Open in Session"]
        let field = dialog.textFields.firstMatch
        XCTAssertTrue(field.waitForExistence(timeout: 5))
        field.tap()
        field.typeText("Summarize next steps")
        dialog.buttons["Open Session"].tap()
        XCTAssertTrue(app.alerts.staticTexts["Opened a Claude thread in T3 on the mini."].waitForExistence(timeout: 8))
    }

    func testInboxContextMenuNotifyMeToggles() {
        XCUIDevice.shared.orientation = .portrait
        launch()
        let row = app.buttons["message-10"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.65)).press(forDuration: 1)
        let notify = app.buttons["Notify Me"].firstMatch
        XCTAssertTrue(notify.waitForExistence(timeout: 5))
        capture("Context menu with Notify Me")
        notify.tap()
        XCTAssertTrue(app.alerts.staticTexts["You’ll be notified of replies"].waitForExistence(timeout: 5))
        app.alerts.buttons["OK"].tap()
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.65)).press(forDuration: 1)
        let stop = app.buttons["Stop Notifying"].firstMatch
        XCTAssertTrue(stop.waitForExistence(timeout: 5), "A watched thread offers Stop Notifying")
        XCTAssertFalse(app.buttons["Notify Me"].exists)
        stop.tap()
        XCTAssertTrue(app.alerts.staticTexts["Reply notifications off"].waitForExistence(timeout: 5))
        app.alerts.buttons["OK"].tap()
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.65)).press(forDuration: 1)
        XCTAssertTrue(app.buttons["Notify Me"].firstMatch.waitForExistence(timeout: 5))
    }

    func testFixedWidthEmailFitsClosedReaderAndRotation() {
        XCUIDevice.shared.orientation = .portrait
        launch(["--show-reader", "--fixed-width-email"])
        let web = app.webViews.firstMatch
        let edge = web.staticTexts["Right edge"]
        XCTAssertTrue(edge.waitForExistence(timeout: 15))
        capture("600px email in portrait reader")
        XCTAssertTrue(edge.isHittable, "The right edge of a fixed-width email must be visible")
        XCTAssertLessThanOrEqual(edge.frame.maxX, web.frame.maxX + 1)
        XCUIDevice.shared.orientation = .landscapeLeft
        XCTAssertTrue(edge.waitForExistence(timeout: 10))
        XCTAssertLessThanOrEqual(edge.frame.maxX, web.frame.maxX + 1, "Resize must refit the HTML")
        capture("600px email after width change")
        XCUIDevice.shared.orientation = .portrait
        let footer = web.staticTexts["Bottom of the complete email"]
        let toolbar = app.buttons["readerArchive"]
        for _ in 0..<4 {
            if footer.isHittable, footer.frame.maxY < toolbar.frame.minY { break }
            app.scrollViews.firstMatch.swipeUp()
        }
        XCTAssertTrue(footer.isHittable, "Fitting must preserve the complete document height")
        XCTAssertLessThan(footer.frame.maxY, toolbar.frame.minY, "The footer must scroll clear of the floating toolbar")
        capture("Complete fixed-width email footer")
    }
    func testTrustedSenderImagesAndAuthenticationDetails() {
        launch(["--image-policy", "--show-reader"])
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["senderAuthentication"].waitForExistence(timeout: 5))
        app.buttons["senderAuthentication"].tap()
        XCTAssertTrue(app.staticTexts["Domain authenticated: example.org"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["showImages"].exists)
        app.buttons["senderImageSettings"].tap()
        app.buttons["Always trust this sender"].tap()
        let hidden = NSPredicate(format: "exists == false")
        expectation(for: hidden, evaluatedWith: app.buttons["showImages"])
        waitForExpectations(timeout: 10)
        capture("Authenticated sender images load automatically")
        app.buttons["senderImageSettings"].tap()
        app.buttons["Stop trusting this sender"].tap()
        XCTAssertTrue(app.buttons["showImages"].waitForExistence(timeout: 10))
        capture("Revoked sender image trust")
    }
    func testFailedAuthenticationCannotTrustSenderAutomatically() {
        launch(["--image-policy", "--image-auth-failed", "--show-reader"])
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["showImages"].waitForExistence(timeout: 5))
        app.buttons["senderImageSettings"].tap()
        let trust = app.buttons["Always trust this sender"]
        XCTAssertTrue(trust.waitForExistence(timeout: 5))
        XCTAssertFalse(trust.isEnabled)
        capture("Failed authentication keeps images blocked")
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
        let body = app.textViews["composeBody"]
        XCTAssertTrue(body.waitForExistence(timeout: 5))
        body.tap(); body.typeText("Tuesday works well. See you then!")
        capture("Reply with keyboard")
        app.buttons["cancelCompose"].tap()
        app.buttons["Keep on this iPhone"].tap()
        app.terminate()
        launch(reset: false)
        let recover = app.buttons["recoverDraft"]
        XCTAssertTrue(recover.waitForExistence(timeout: 10)); recover.tap()
        XCTAssertEqual(app.textViews["composeBody"].value as? String, "Tuesday works well. See you then!")
        capture("Recovered reply")
    }
    func testSmartNotificationSettingsStayInsideFixture() {
        launch()
        if !app.buttons["Settings"].exists { app.buttons["showInbox"].tap() }
        app.buttons["Settings"].tap()
        let enable = app.buttons["Enable smart notifications"]
        for _ in 0..<3 { if enable.isHittable { break }; app.swipeUp() }
        XCTAssertTrue(enable.waitForExistence(timeout: 5))
        enable.tap()
        XCTAssertEqual(app.alerts.count,0,"Fixture must never request real push authorization")
        let sessionAction = app.switches["Open in Session action"]
        XCTAssertTrue(sessionAction.exists)
        for _ in 0..<2 { if sessionAction.isHittable { break }; app.swipeUp() }
        XCTAssertTrue(sessionAction.isHittable)
        XCTAssertEqual(sessionAction.value as? String, "0")
        // SwiftUI exposes the entire labeled row as a switch. Tap its trailing control.
        sessionAction.coordinate(withNormalizedOffset: CGVector(dx: 0.92, dy: 0.5)).tap()
        capture("Session action enabled")
        XCTAssertEqual(sessionAction.value as? String, "1")
        let sender=app.textFields["Trusted sender email"]
        XCTAssertTrue(sender.exists)
        for _ in 0..<2 { if sender.isHittable { break }; app.swipeUp() }
        sender.tap(); sender.typeText("trusted@example.org")
        XCTAssertFalse(app.buttons["openChat"].exists, "Mailbox actions must not leak through the Settings sheet")
        XCTAssertFalse(app.buttons["compose"].exists)
        capture("Smart notification settings")
    }
    func testAppearanceSelectionPersistsAcrossRelaunch() {
        launch()
        if !app.buttons["Settings"].exists { app.buttons["showInbox"].tap() }
        app.buttons["Settings"].tap()
        let picker = app.segmentedControls["appearancePicker"]
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        picker.buttons["Dark"].tap()
        XCTAssertTrue(picker.buttons["Dark"].isSelected)
        capture("Dark appearance settings")
        app.buttons["Done"].tap()
        app.terminate()
        launch(reset: false)
        if !app.buttons["Settings"].exists { app.buttons["showInbox"].tap() }
        app.buttons["Settings"].tap()
        XCTAssertTrue(picker.waitForExistence(timeout: 5))
        XCTAssertTrue(picker.buttons["Dark"].isSelected)
        picker.buttons["Light"].tap()
        capture("Light appearance settings")
        XCTAssertTrue(picker.buttons["Light"].isSelected)
        picker.buttons["System"].tap()
        XCTAssertTrue(picker.buttons["System"].isSelected)
        capture("System appearance settings")
    }
    func testSingleTapBlankRowAreaOpensWhileReadAcknowledgementIsPending() {
        launch(["--slow-read"])
        let row = app.buttons["message-12"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        // Right of the short subject, between the text lines and bottom inset.
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.75)).tap()
        capture("After single row tap")
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.scrollViews.firstMatch.isEnabled, "Reading must remain interactive during mark-read")
        app.scrollViews.firstMatch.swipeUp()
        capture("Reader while mark-read is pending")
        app.buttons["BackButton"].tap()
        XCTAssertTrue(row.waitForExistence(timeout: 5))
        XCTAssertTrue(row.isEnabled, "A pending read acknowledgement must not block navigation")
        XCTAssertFalse(app.buttons["retryConnection"].waitForExistence(timeout: 2),
                       "Leaving the reader cancels its mark-read; that is not a connection failure")
        row.coordinate(withNormalizedOffset: CGVector(dx: 0.60, dy: 0.75)).tap()
        XCTAssertTrue(app.staticTexts["readerSubject"].waitForExistence(timeout: 5))
        capture("Reopened with one tap during mark-read")
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
        app.textViews["composeBody"].tap(); app.textViews["composeBody"].typeText("Tuesday works well.")
        let review = app.buttons["reviewCompose"]
        XCTAssertTrue(review.isHittable, "Review must stay available while typing")
        let hide = app.buttons["hideComposeKeyboard"]
        XCTAssertTrue(hide.isHittable)
        XCTAssertFalse(app.buttons["openChat"].exists, "Chat must hide while the keyboard is exposed")
        XCTAssertFalse(app.buttons["compose"].exists, "New message must hide while the keyboard is exposed")
        XCTAssertLessThanOrEqual(hide.frame.maxY, app.keyboards.firstMatch.frame.minY + 1)
        capture("Composer with keyboard before review")
        hide.tap()
        expectation(for: NSPredicate(format: "exists == false"), evaluatedWith: app.keyboards.firstMatch)
        waitForExpectations(timeout: 5)
        XCTAssertEqual(app.textViews["composeBody"].value as? String, "Tuesday works well.")
        app.textViews["composeBody"].tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(review.isHittable)
        review.tap()
        let send = app.buttons["sendReviewedDraft"]
        XCTAssertTrue(send.waitForExistence(timeout: 10))
        XCTAssertTrue(app.staticTexts["copy@example.org"].exists)
        capture("All recipients review")
        send.tap()
        XCTAssertTrue(app.buttons["Send now"].waitForExistence(timeout: 5))
        confirmSend()
        XCTAssertTrue(app.alerts.staticTexts["Sent."].waitForExistence(timeout: 10))
    }

    func testComposerShowsSignatureBelowWritingArea() {
        XCUIDevice.shared.orientation = .portrait
        launch()
        if app.frame.width >= 669 {
            app.terminate()
            XCUIDevice.shared.orientation = .landscapeLeft
            launch(["--duo-flat"])
        }
        let compose = app.buttons["compose"]
        if !compose.exists, app.buttons["showInbox"].exists { app.buttons["showInbox"].tap() }
        XCTAssertTrue(compose.waitForExistence(timeout: 10))
        compose.tap()
        let body = app.textViews["composeBody"]
        let signature = app.webViews.staticTexts["Morgan Reese"]
        XCTAssertTrue(signature.waitForExistence(timeout: 10))
        XCTAssertLessThan(body.frame.minY, signature.frame.minY)
        XCTAssertTrue(app.webViews.staticTexts["Bluebird Studio"].exists)
        XCTAssertTrue(app.buttons["saveDraft"].exists)
        capture("Writing area and desktop signature")
        let from = app.buttons["composeFrom"]
        from.tap()
        let personal = app.buttons["personal@example.com"]
        XCTAssertTrue(personal.waitForExistence(timeout: 5))
        personal.tap()
        XCTAssertFalse(signature.exists)
        from.tap()
        app.buttons["me@example.com"].tap()
        XCTAssertTrue(signature.waitForExistence(timeout: 5))
        app.buttons["showComposeCopies"].tap()
        XCTAssertTrue(app.textFields["composeCc"].exists)
        XCTAssertTrue(app.textFields["composeBcc"].exists)
    }

    func testComposerAtLargestTextSize() {
        XCUIDevice.shared.orientation = .portrait
        launch(["-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryAccessibilityXXXL"])
        let compose = app.buttons["compose"]
        XCTAssertTrue(compose.waitForExistence(timeout: 10))
        compose.tap()
        let to = app.textFields["composeTo"]
        XCTAssertTrue(to.waitForExistence(timeout: 10))
        XCTAssertGreaterThan(to.frame.height, 35, "The large text override must reach the composer")
        XCTAssertGreaterThan(to.frame.width, 210, "Leave room to type with Cc/Bcc below")
        let from = app.buttons["composeFrom"]
        XCTAssertTrue(from.exists)
        XCTAssertLessThanOrEqual(from.frame.maxY, to.frame.minY)
        capture("Composer at accessibility XXXL")
    }
    func testEditSavedDraftFromReview() {
        let fixture = ["--draft-attachment-fixture"]
        launch(fixture)
        if app.frame.width >= 669 {
            app.terminate()
            XCUIDevice.shared.orientation = .landscapeLeft
            launch(fixture + ["--duo-flat"])
        }
        tapDraftsTab()
        let draft = app.buttons["message-20"]
        XCTAssertTrue(draft.waitForExistence(timeout: 10))
        draft.tap()
        let edit = app.buttons["editReviewedDraft"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10))
        edit.tap()
        XCTAssertTrue(app.navigationBars["Edit draft"].waitForExistence(timeout: 10))
        let subject = app.textFields["composeSubject"]
        XCTAssertTrue(subject.waitForExistence(timeout: 10))
        capture("Editing a saved draft")
        subject.tap()
        subject.typeText(" tomorrow")
        let changedSubject = subject.value as? String ?? ""
        XCTAssertTrue(changedSubject.contains("tomorrow"))
        app.buttons["reviewCompose"].tap()
        XCTAssertTrue(app.staticTexts[changedSubject].waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["sendReviewedDraft"].waitForExistence(timeout: 15))
        XCTAssertEqual(app.buttons["reviewAttachment-0"].label, "Study-planner.docx")
        XCTAssertTrue(app.webViews.staticTexts["Hi Chris,"].waitForExistence(timeout: 15))
    }

    func testPastePhotoIntoSavedDraft() {
        launch(["--draft-attachment-fixture", "--paste-photo"])
        tapDraftsTab()
        let draft = app.buttons["message-20"]
        XCTAssertTrue(draft.waitForExistence(timeout: 10)); draft.tap()
        let edit = app.buttons["editReviewedDraft"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10)); edit.tap()
        XCTAssertTrue(app.navigationBars["Edit draft"].waitForExistence(timeout: 10))
        let body = app.textViews["composeBody"]
        XCTAssertTrue(body.waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["addPhoto"].exists && app.buttons["addFile"].exists, "Saved drafts take attachments too")
        pasteFromMenu(into: body)
        XCTAssertTrue(app.descendants(matching: .any)["composePhoto"].waitForExistence(timeout: 5))
        capture("Saved draft with pasted photo")
        app.buttons["reviewCompose"].tap()
        XCTAssertTrue(app.buttons["sendReviewedDraft"].waitForExistence(timeout: 15), "The fixture refuses an edit without exactly one valid photo")
        XCTAssertEqual(app.buttons["reviewAttachment-0"].label, "Study-planner.docx", "Existing files stay on the draft")
    }

    func testSaveFollowsANewerRevisionAndKeepsPhotos() {
        launch(["--draft-attachment-fixture", "--paste-photo", "--newer-revision"])
        tapDraftsTab()
        let draft = app.buttons["message-20"]
        XCTAssertTrue(draft.waitForExistence(timeout: 10)); draft.tap()
        let edit = app.buttons["editReviewedDraft"]
        XCTAssertTrue(edit.waitForExistence(timeout: 10)); edit.tap()
        let body = app.textViews["composeBody"]
        XCTAssertTrue(body.waitForExistence(timeout: 10))
        pasteFromMenu(into: body)
        XCTAssertTrue(app.descendants(matching: .any)["composePhoto"].waitForExistence(timeout: 5))
        app.buttons["reviewCompose"].tap()
        let keep = app.buttons["keepLatestText"].firstMatch
        XCTAssertTrue(keep.waitForExistence(timeout: 10), "A replaced revision asks which text to keep instead of dead-ending")
        XCTAssertTrue(app.buttons["useMyText"].firstMatch.exists)
        capture("Newer revision choice")
        keep.tap()
        XCTAssertTrue(app.buttons["sendReviewedDraft"].waitForExistence(timeout: 15),
                      "The fixture only accepts revision 21 with its latest text and exactly one photo")
    }

    func testDraftAttachmentPreviewAndExpiredReview() {
        var fixture = ["--draft-attachment-fixture", "--word-page-fixture"]
        launch(fixture)
        if app.frame.width >= 669 {
            app.terminate()
            XCUIDevice.shared.orientation = .landscapeLeft
            fixture.append("--duo-flat")
            launch(fixture)
        }
        tapDraftsTab()
        let row = app.buttons["message-20"]
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        let attachment = app.buttons["reviewAttachment-0"]
        XCTAssertTrue(attachment.waitForExistence(timeout: 15))
        XCTAssertEqual(attachment.label, "Study-planner.docx")
        capture("Clickable Word attachment tile")
        attachment.tap()
        let page = app.descendants(matching: .any).matching(identifier: "wordPagePreview").firstMatch
        XCTAssertTrue(page.waitForExistence(timeout: 15))
        XCTAssertTrue(app.buttons["shareDraftAttachment"].isEnabled)
        capture("Paginated Word attachment")
        app.buttons["closeDraftAttachment"].tap()
        XCTAssertTrue(app.buttons["sendReviewedDraft"].isEnabled)

        app.terminate()
        launch(fixture + ["--attachment-review-expired"])
        tapDraftsTab()
        XCTAssertTrue(row.waitForExistence(timeout: 10))
        row.tap()
        XCTAssertTrue(attachment.waitForExistence(timeout: 15))
        attachment.tap()
        XCTAssertTrue(app.staticTexts["sendError"].waitForExistence(timeout: 10))
        XCTAssertFalse(app.buttons["sendReviewedDraft"].isEnabled)
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
        let input = app.textViews["chatInput"]; XCTAssertTrue(input.waitForExistence(timeout: 10))
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
    /// Paper canvas and the T3-style chat in light mode, for visual review.
    func testLightPaperInboxAndChat() {
        launch(["-fixtureAppearance", "light", "--chat-rich"])
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout: 10))
        capture("Light paper inbox")
        app.buttons["openChat"].tap()
        let input = app.textViews["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Write a short plan")
        app.buttons["chatSend"].tap()
        XCTAssertTrue(app.staticTexts["Next steps"].waitForExistence(timeout: 10))
        app.scrollViews.containing(.staticText, identifier: "Next steps").firstMatch.swipeDown()
        capture("Light paper chat")
    }

    func testChatMarkdownAndComposer() {
        launch(["--chat-rich"])
        app.buttons["openChat"].tap()
        let input = app.textViews["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("Write a short plan")
        capture("Chat multiline composer")
        app.buttons["chatSend"].tap()
        XCTAssertTrue(app.staticTexts["Next steps"].waitForExistence(timeout: 10))
        app.scrollViews.containing(.staticText, identifier: "Next steps").firstMatch.swipeDown()
        XCTAssertTrue(app.staticTexts["Reply to Jordan"].exists)
        XCTAssertTrue(app.staticTexts["Tuesday works well."].exists)
        XCTAssertFalse(app.staticTexts["## Next steps"].exists)
        let work = app.buttons["chatWorkGroup"]
        XCTAssertTrue(work.exists, "Tool calls fold into one work-log row")
        XCTAssertTrue(work.label.contains("Searched mail, read 2 emails"), work.label)
        XCTAssertEqual(app.buttons.matching(identifier: "chatWorkGroup").count, 1)
        work.tap()
        XCTAssertTrue(app.staticTexts["Read an email"].waitForExistence(timeout: 3), "Expanding shows each call")
        capture("Chat work log expanded")
        XCTAssertEqual(app.buttons["chatSend"].label, "Send message")
        capture("Formatted chat transcript")
        app.buttons["chatContext"].tap()
        XCTAssertFalse(app.buttons["Current email"].isEnabled, "Email context requires an open message")
        app.buttons["Your mail"].tap()
    }
    /// Paste the way a person does: long-press in the text, then Paste from the edit menu.
    func pasteFromMenu(into field: XCUIElement) {
        if !app.keyboards.firstMatch.exists { field.tap(); _ = app.keyboards.firstMatch.waitForExistence(timeout: 3) }
        field.press(forDuration: 1.2)
        let menuItem = app.menuItems["Paste"]
        let button = app.buttons["Paste"]
        if menuItem.waitForExistence(timeout: 3) { menuItem.tap() }
        else { XCTAssertTrue(button.waitForExistence(timeout: 2), "Paste must be offered when the clipboard holds an image"); button.tap() }
    }
    func testPastedPhotoGoesWithEmailAndChat() {
        launch(["--paste-photo"])
        XCTAssertTrue(app.buttons["compose"].waitForExistence(timeout: 10)); app.buttons["compose"].tap()
        let to = app.textFields["composeTo"]; XCTAssertTrue(to.waitForExistence(timeout: 5)); to.tap(); to.typeText("jordan@example.org")
        let body = app.textViews["composeBody"]
        body.tap(); body.typeText("Here is the photo.")
        XCTAssertTrue(app.buttons["addPhoto"].exists); XCTAssertTrue(app.buttons["addFile"].exists)
        let photo = app.descendants(matching: .any)["composePhoto"]
        pasteFromMenu(into: body)
        XCTAssertTrue(photo.waitForExistence(timeout: 5))
        XCTAssertEqual(body.value as? String, "Here is the photo.", "An image paste must not change the text")
        capture("Composer with pasted photo")
        app.buttons["removePhoto"].firstMatch.tap()
        XCTAssertFalse(photo.waitForExistence(timeout: 1))
        pasteFromMenu(into: body)
        XCTAssertTrue(photo.waitForExistence(timeout: 5))
        XCTAssertEqual(app.descendants(matching: .any).matching(identifier: "composePhoto").count, 1)
        app.buttons["reviewCompose"].tap()
        XCTAssertTrue(app.buttons["sendReviewedDraft"].waitForExistence(timeout: 10), "The fixture refuses a draft without exactly one valid photo")

        app.terminate(); launch(["--paste-photo", "--show-reader"])
        XCTAssertTrue(app.buttons["readerChat"].waitForExistence(timeout: 10)); app.buttons["readerChat"].tap()
        let input = app.textViews["chatInput"]
        XCTAssertTrue(input.waitForExistence(timeout: 10))
        input.tap(); input.typeText("What is in this picture?")
        let chatPhoto = app.descendants(matching: .any)["composePhoto"]
        pasteFromMenu(into: input)
        XCTAssertTrue(chatPhoto.waitForExistence(timeout: 5))
        capture("Chat with pasted photo")
        app.buttons["chatSend"].tap()
        // The turn is appended before the request; check it before the reply scrolls it away.
        XCTAssertTrue(app.images["chatSentPhoto"].waitForExistence(timeout: 2.5), "The sent turn shows its photo")
        capture("Chat turn with photo")
        XCTAssertTrue(app.buttons["permissionAllow"].waitForExistence(timeout: 10),
                      "The fixture only answers a message carrying exactly one valid photo")
        XCTAssertFalse(app.staticTexts.containing(NSPredicate(format: "label CONTAINS %@", "Couldn’t confirm delivery")).firstMatch.exists)
        XCTAssertFalse(chatPhoto.exists, "Sent photos leave the input")
    }
    func testChatContextPickerKeepsConversationAndDraft() {
        launch(["--show-reader", "--chat-rich"])
        XCTAssertTrue(app.buttons["readerChat"].waitForExistence(timeout: 10)); app.buttons["readerChat"].tap()
        let input = app.textViews["chatInput"]
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
        let input = app.textViews["chatInput"]
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
        capture("Docked chat with keyboard")
        XCTAssertTrue(divider.exists, "Showing the keyboard must keep split chat docked")
        XCTAssertTrue(input.isHittable,"Docked input must remain reachable with the keyboard open")
        XCTAssertLessThanOrEqual(input.frame.maxY,app.keyboards.firstMatch.frame.minY,"Docked composer must sit above the software keyboard")
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
    func tapSearchTab() {
        let tabs = app.buttons.matching(identifier: "Search")
        XCTAssertTrue(tabs.firstMatch.waitForExistence(timeout: 5))
        let nested = tabs.firstMatch.buttons.matching(identifier: "Search")
        if nested.count == 1 { nested.element.tap() }
        else { XCTAssertEqual(tabs.count,1); tabs.element.tap() }
    }
    func testSearchKeyboardInActualWideWindow() {
        launch(["-fixtureAppearance", "light", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryXXXL"])
        XCUIDevice.shared.orientation = .portrait
        XCUIDevice.shared.orientation = .landscapeLeft
        XCTAssertGreaterThan(app.frame.width,867,"Use the iPad's real landscape window for keyboard avoidance")
        tapSearchTab()
        let search=app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout:5))
        XCTAssertTrue(app.otherElements["detailPane"].exists)
        search.tap(); search.typeText("project")
        XCTAssertEqual(search.value as? String,"project")
        capture("Real wide window after typing")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(app.otherElements["detailPane"].exists,"The keyboard must not replace the split navigator")
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout:10))
        search.typeText(" notes")
        XCTAssertEqual(search.value as? String,"project notes")
        capture("Real wide window with focused search")
        assertSearchKeyboardCanCollapse(expectSideRail: true)
    }
    func assertSearchKeyboardCanCollapse(expectSideRail: Bool = false) {
        let search=app.searchFields.firstMatch
        let hide=app.buttons["hideSearchKeyboard"]
        XCTAssertTrue(hide.waitForExistence(timeout:5))
        XCTAssertTrue(hide.isHittable)
        XCTAssertFalse(app.buttons["openChat"].exists, "Chat must hide while the keyboard is exposed")
        XCTAssertFalse(app.buttons["compose"].exists, "New message must hide while the keyboard is exposed")
        XCTAssertLessThanOrEqual(hide.frame.maxY,app.keyboards.firstMatch.frame.minY+1)
        if expectSideRail {
            XCTAssertGreaterThan(hide.frame.midX, app.frame.midX, "Keyboard dismissal belongs on the trailing side of the whole window")
            XCTAssertGreaterThan(hide.frame.midX, app.buttons["Settings"].frame.maxX,
                                 "The Duo must place keyboard dismissal in its right rail, outside the account bar")
        }
        capture("Search keyboard collapse in native toolbar")
        let query=search.value as? String
        hide.tap()
        let gone=NSPredicate(format:"exists == false")
        expectation(for:gone,evaluatedWith:app.keyboards.firstMatch)
        waitForExpectations(timeout:5)
        XCTAssertTrue(app.buttons["openChat"].waitForExistence(timeout: 5))
        XCTAssertTrue(app.buttons["compose"].exists)
        XCTAssertEqual(search.value as? String,query,"Hiding keyboard must retain the search query")
        XCTAssertTrue(app.buttons["message-10"].exists,"Hiding keyboard must retain the results")
        XCTAssertFalse(app.staticTexts["readerSubject"].exists,"Keyboard dismissal must not open a message underneath the rail")
        capture("Search results with keyboard collapsed")
        search.tap()
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout:5))
        XCTAssertEqual(search.value as? String,query)
    }
    func testSearchAndAccountSelection() {
        launch()
        tapSearchTab()
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5)); search.tap(); search.typeText("project")
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout: 10))
        capture("Search")
        assertSearchKeyboardCanCollapse(expectSideRail: true)
    }
    func testWideSearchKeepsKeyboardAndTypedQuery() {
        XCUIDevice.shared.orientation = .landscapeLeft
        launch(["--duo-wide", "-fixtureAppearance", "light", "-UIPreferredContentSizeCategoryName", "UICTContentSizeCategoryXXXL"])
        let tabs = app.buttons.matching(identifier: "Search")
        XCTAssertTrue(tabs.firstMatch.waitForExistence(timeout: 5))
        let nested = tabs.firstMatch.buttons.matching(identifier: "Search")
        if nested.count == 1 { nested.element.tap() } else { tabs.firstMatch.tap() }
        let search = app.searchFields.firstMatch
        XCTAssertTrue(search.waitForExistence(timeout: 5))
        let pane = app.otherElements["detailPane"]
        XCTAssertTrue(pane.exists)
        search.tap()
        search.typeText("pro")
        XCTAssertEqual(search.value as? String, "pro")
        XCTAssertTrue(app.keyboards.firstMatch.waitForExistence(timeout: 5))
        XCTAssertTrue(pane.exists, "Keyboard must not replace the split navigator")
        search.typeText("ject")
        XCTAssertEqual(search.value as? String, "project")
        XCTAssertTrue(app.buttons["message-10"].waitForExistence(timeout: 10))
        search.typeText(" notes")
        XCTAssertEqual(search.value as? String, "project notes", "Results must not steal focus")
        XCTAssertTrue(app.staticTexts["searchStatus"].waitForExistence(timeout: 10))
        XCTAssertTrue(app.buttons["message-10"].exists)
        capture("Wide search with keyboard and continuous typing")
        // The iPad can pan this fixed-size viewport's native toolbar offscreen.
        // Its touch/keyboard checks run in the actual wide window at XXXL too.
        XCTAssertTrue(app.buttons["hideSearchKeyboard"].exists)
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
        let body = app.textViews["composeBody"]
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
