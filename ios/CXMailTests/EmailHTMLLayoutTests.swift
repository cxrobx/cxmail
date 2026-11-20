import XCTest
import WebKit
@testable import CXMail

@MainActor
private final class DocumentLoaded: NSObject, WKNavigationDelegate {
    let expectation: XCTestExpectation
    init(_ expectation: XCTestExpectation) { self.expectation = expectation }
    func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) { expectation.fulfill() }
}

@MainActor
final class EmailHTMLLayoutTests: XCTestCase {
    private func render(_ html: String, width: CGFloat = 320, in window: UIWindow? = nil) async throws -> WKWebView {
        let web = WKWebView(frame: CGRect(x: 0, y: 0, width: width, height: 240), configuration: EmailHTMLView.webConfiguration())
        window?.addSubview(web)
        let loaded = expectation(description: "Native WebKit loaded the email")
        let delegate = DocumentLoaded(loaded)
        web.navigationDelegate = delegate
        web.layoutIfNeeded()
        web.loadHTMLString(EmailHTMLView.document(html, images: false), baseURL: URL(string: "about:blank"))
        await fulfillment(of: [loaded], timeout: 15)
        web.layoutIfNeeded()
        withExtendedLifetime(delegate) {}
        web.navigationDelegate = nil
        return web
    }
    private func fit(_ web: WKWebView) async throws -> [String: Double] {
        let result = try await web.evaluateJavaScript(EmailHTMLView.fittingScript, in: nil, contentWorld: .defaultClient)
        return try XCTUnwrap(result as? [String: Double])
    }
    func testFixedWidthTableFitsAndItsFooterRemainsInsideTheDocument() async throws {
        let web = try await render(Fixtures.fixedWidthHTML)
        let metrics = try await fit(web)
        XCTAssertGreaterThanOrEqual(try XCTUnwrap(metrics["natural"]), 600)
        XCTAssertLessThan(try XCTUnwrap(metrics["scale"]), 1)
        let result = try await web.evaluateJavaScript("""
        (() => { const v=document.getElementById('cxmail-native-viewport').getBoundingClientRect();
          const t=document.querySelector('table').getBoundingClientRect();
          const p=[...document.querySelectorAll('p')].at(-1).getBoundingClientRect();
          return {right:t.right, left:t.left, viewportRight:v.right, viewportLeft:v.left,
            footerBottom:p.bottom, viewportBottom:v.bottom, pageWidth:document.documentElement.scrollWidth}; })()
        """, in: nil, contentWorld: .defaultClient)
        let rect = try XCTUnwrap(result as? [String: Double])
        XCTAssertLessThanOrEqual(try XCTUnwrap(rect["right"]), try XCTUnwrap(rect["viewportRight"]) + 1)
        XCTAssertGreaterThanOrEqual(try XCTUnwrap(rect["left"]), try XCTUnwrap(rect["viewportLeft"]) - 1)
        XCTAssertLessThanOrEqual(try XCTUnwrap(rect["footerBottom"]), try XCTUnwrap(rect["viewportBottom"]) + 1)
        XCTAssertLessThanOrEqual(try XCTUnwrap(rect["pageWidth"]), 321)
    }
    /// Wide quoted Outlook mail: iOS text autosizing inflated the paragraphs differently on
    /// alternate fitting passes, so the measured height flipped every refit and the reader
    /// jittered between two layouts. Repeated passes must measure the same document.
    func testRepeatedFittingOfWideQuotedMailIsStable() async throws {
        let url = try XCTUnwrap(Bundle(for: Self.self).url(forResource: "wide-quoted-outlook", withExtension: "html"))
        let html = try String(contentsOf: url, encoding: .utf8)
        // Autosizing only runs for a web view that is on screen.
        let window = UIWindow(frame: CGRect(x: 0, y: 0, width: 600, height: 900))
        window.makeKeyAndVisible()
        let web = try await render(html, width: 353, in: window)
        defer { web.removeFromSuperview(); window.isHidden = true }
        try await Task.sleep(nanoseconds: 300_000_000)
        var heights: [Double] = []
        var scale = 1.0
        for _ in 0..<4 {
            let metrics = try await fit(web)
            heights.append(try XCTUnwrap(metrics["height"])); scale = try XCTUnwrap(metrics["scale"])
        }
        XCTAssertLessThan(scale, 1, "fixture must exercise the scaled path")
        XCTAssertEqual(Set(heights).count, 1, "fitting passes disagree: \(heights)")
    }
    func testResponsiveMailIsNotScaledDown() async throws {
        let web = try await render("<p>Ordinary correspondence wraps at the available width.</p><p>All of this email remains readable.</p>")
        let metrics = try await fit(web)
        XCTAssertEqual(try XCTUnwrap(metrics["scale"]), 1, accuracy: 0.001)
    }
    func testWideReaderDoesNotEnlargeOrShrinkTheOriginalLayout() async throws {
        let web = try await render(Fixtures.fixedWidthHTML, width: 760)
        let metrics = try await fit(web)
        XCTAssertEqual(try XCTUnwrap(metrics["scale"]), 1, accuracy: 0.001)
    }
    func testFittingKeepsEmailScriptsDisabledAndImagesBlocked() async throws {
        let web = try await render("<script>window.emailScriptRan=true</script><p>Safe text</p><img src='https://blocked.invalid/image.png'>")
        _ = try await fit(web)
        XCTAssertFalse(web.configuration.defaultWebpagePreferences.allowsContentJavaScript)
        XCTAssertFalse(web.configuration.websiteDataStore.isPersistent)
        let marker = try await web.evaluateJavaScript("typeof window.emailScriptRan", in: nil, contentWorld: .page)
        XCTAssertEqual(marker as? String, "undefined")
        let policies = try await web.evaluateJavaScript("[...document.querySelectorAll('meta[http-equiv]')].map(m=>m.content).join(';')", in: nil, contentWorld: .defaultClient)
        let csp = try XCTUnwrap(policies as? String)
        XCTAssertTrue(csp.contains("default-src 'none'"))
        XCTAssertTrue(csp.contains("img-src data: cid:;"))
        XCTAssertTrue(csp.contains("connect-src 'none'"))
    }
}
