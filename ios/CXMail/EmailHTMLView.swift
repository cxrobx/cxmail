import SwiftUI
import WebKit

/// Sanitized server HTML stays a document, never app code. No scripts, storage,
/// forms, frames or remote resources without an explicit image opt-in.
struct EmailHTMLView: UIViewRepresentable {
    let html: String
    var images = false
    @Binding var height: CGFloat
    func makeCoordinator() -> Coordinator { Coordinator(height: $height) }
    func makeUIView(context: Context) -> WKWebView {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.preferences.javaScriptCanOpenWindowsAutomatically = false
        let view = WKWebView(frame: .zero, configuration: config)
        view.navigationDelegate = context.coordinator
        view.isOpaque = false
        view.backgroundColor = .white
        view.scrollView.isScrollEnabled = false
        view.scrollView.contentInsetAdjustmentBehavior = .never
        view.accessibilityLabel = "Email body"
        context.coordinator.observation = view.scrollView.observe(\.contentSize, options: [.new]) { _, change in
            guard let size = change.newValue, size.height.isFinite else { return }
            Task { @MainActor in context.coordinator.height.wrappedValue = max(180, min(30_000, size.height)) }
        }
        return view
    }
    func updateUIView(_ view: WKWebView, context: Context) {
        let input = (images ? "images:" : "blocked:") + html
        guard context.coordinator.loaded != input else { return }
        context.coordinator.loaded = input
        let img = images ? "data: cid: https:" : "data: cid:"
        let policy = "default-src 'none'; img-src \(img); style-src 'unsafe-inline'; font-src data:; base-uri 'none'; form-action 'none'; frame-src 'none'; connect-src 'none'"
        let head = "<meta http-equiv=\"Content-Security-Policy\" content=\"\(policy)\"><meta name=\"referrer\" content=\"no-referrer\"><meta name=\"viewport\" content=\"width=device-width,initial-scale=1\"><style>:root{color-scheme:light}html,body{overflow-wrap:anywhere}img,table{max-width:100%!important}pre{white-space:pre-wrap}</style>"
        view.loadHTMLString(head + html, baseURL: URL(string: "about:blank"))
    }
    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.observation?.invalidate(); view.stopLoading(); view.navigationDelegate = nil
    }
    final class Coordinator: NSObject, WKNavigationDelegate {
        var height: Binding<CGFloat>
        var observation: NSKeyValueObservation?
        var loaded = ""
        init(height: Binding<CGFloat>) { self.height = height }
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
            #if DEBUG
            print("CXMail document navigation: \(navigationAction.navigationType.rawValue), scheme \(navigationAction.request.url?.scheme ?? "nil")")
            #endif
            guard let url = navigationAction.request.url else {
                decisionHandler(navigationAction.navigationType == .other ? .allow : .cancel); return
            }
            if navigationAction.navigationType == .linkActivated,
               ["https", "http", "mailto", "tel"].contains(url.scheme?.lowercased() ?? "") {
                UIApplication.shared.open(url)
                decisionHandler(.cancel); return
            }
            decisionHandler(url.scheme == "about" && navigationAction.navigationType == .other ? .allow : .cancel)
        }
        func webView(_ webView: WKWebView, didFinish navigation: WKNavigation!) {
            #if DEBUG
            print("CXMail document loaded; height \(webView.scrollView.contentSize.height)")
            #endif
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            #if DEBUG
            print("CXMail document failed: \((error as NSError).domain) \((error as NSError).code)")
            #endif
        }
    }
}
