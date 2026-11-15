import SwiftUI
import WebKit

/// Own the viewport size; an email's fixed table width must never become the
/// native view's width. Content scripts stay disabled, including while fitting.
final class EmailDocumentWebView: WKWebView {
    var onWidthChange: ((WKWebView) -> Void)?
    private var lastWidth: CGFloat = 0
    override func layoutSubviews() {
        super.layoutSubviews()
        if bounds.width > 0, abs(bounds.width - lastWidth) > 0.5 {
            lastWidth = bounds.width
            onWidthChange?(self)
        }
    }
}

struct EmailHTMLView: UIViewRepresentable {
    let html: String
    var images = false
    @Binding var height: CGFloat
    var onLoaded: (() -> Void)? = nil
    func makeCoordinator() -> Coordinator { Coordinator(height: $height, onLoaded: onLoaded) }
    func makeUIView(context: Context) -> WKWebView {
        let coordinator = context.coordinator
        let config = Self.webConfiguration()
        let view = EmailDocumentWebView(frame: .zero, configuration: config)
        view.navigationDelegate = coordinator
        view.isOpaque = false
        view.backgroundColor = .white
        view.scrollView.isScrollEnabled = false
        view.scrollView.contentInsetAdjustmentBehavior = .never
        view.accessibilityLabel = "Email body"
        view.onWidthChange = { [weak coordinator] view in coordinator?.fit(view) }
        coordinator.observation = view.scrollView.observe(\.contentSize, options: [.new]) { [weak coordinator, weak view] _, _ in
            Task { @MainActor in
                guard let view else { return }
                coordinator?.fit(view)
            }
        }
        return view
    }
    static func webConfiguration() -> WKWebViewConfiguration {
        let config = WKWebViewConfiguration()
        config.websiteDataStore = .nonPersistent()
        config.defaultWebpagePreferences.allowsContentJavaScript = false
        config.preferences.javaScriptCanOpenWindowsAutomatically = false
        return config
    }
    func sizeThatFits(_ proposal: ProposedViewSize, uiView: WKWebView, context: Context) -> CGSize? {
        guard let width = proposal.width, width.isFinite, width > 0 else { return nil }
        return CGSize(width: width, height: height)
    }
    func updateUIView(_ view: WKWebView, context: Context) {
        let input = (images ? "images:" : "blocked:") + html
        let coordinator = context.coordinator
        coordinator.onLoaded = onLoaded
        guard coordinator.loaded != input else {
            if coordinator.ready, !view.isLoading { Task { @MainActor in coordinator.onLoaded?() } }
            return
        }
        coordinator.loaded = input
        coordinator.revision += 1
        coordinator.ready = false; coordinator.fitting = false; coordinator.fitAgain = false
        view.loadHTMLString(Self.document(html, images: images), baseURL: URL(string: "about:blank"))
    }
    static func document(_ html: String, images: Bool) -> String {
        let img = images ? "data: cid: https:" : "data: cid:"
        let policy = "default-src 'none'; img-src \(img); style-src 'unsafe-inline'; font-src data:; base-uri 'none'; form-action 'none'; frame-src 'none'; connect-src 'none'"
        return """
        <!doctype html><html><head><meta charset="utf-8">
        <meta http-equiv="Content-Security-Policy" content="\(policy)">
        <meta name="referrer" content="no-referrer"><meta name="viewport" content="width=device-width,initial-scale=1">
        <style>
        :root{color-scheme:light}
        html,body{margin:0!important;min-width:0!important;width:100%!important;max-width:100%!important;box-sizing:border-box!important;overflow-wrap:anywhere}
        body{padding:12px!important}
        img,table{max-width:100%!important}pre{white-space:pre-wrap}
        #cxmail-native-viewport{position:relative;width:100%;min-width:0}
        #cxmail-native-content{display:flow-root;position:relative;width:100%;transform-origin:top left}
        </style></head><body><div id="cxmail-native-viewport"><div id="cxmail-native-content">\(html)</div></div></body></html>
        """
    }
    /// Constant app-authored layout code, evaluated only in the isolated client
    /// world. It never enables email JavaScript or requests remote resources.
    static let fittingScript = """
    (() => {
      const viewport = document.getElementById('cxmail-native-viewport');
      const content = document.getElementById('cxmail-native-content');
      if (!viewport || !content) return null;
      const available = viewport.clientWidth;
      if (!(available > 0)) return null;
      content.style.width = available + 'px';
      content.style.transform = 'none';
      let natural = Math.max(available, content.scrollWidth);
      if (natural > available + 1) content.style.width = natural + 'px';
      natural = Math.max(natural, content.scrollWidth);
      const scale = Math.min(1, available / natural);
      const contentHeight = Math.ceil(Math.max(content.scrollHeight, content.getBoundingClientRect().height) * scale);
      content.style.position = 'absolute';
      content.style.transform = 'scale(' + scale + ')';
      viewport.style.overflow = 'hidden';
      viewport.style.height = contentHeight + 'px';
      const height = Math.ceil(viewport.getBoundingClientRect().top + window.scrollY + contentHeight
        + parseFloat(getComputedStyle(document.body).paddingBottom));
      return { available, natural, scale, height };
    })()
    """
    static func dismantleUIView(_ view: WKWebView, coordinator: Coordinator) {
        coordinator.revision += 1; coordinator.ready = false
        coordinator.observation?.invalidate()
        (view as? EmailDocumentWebView)?.onWidthChange = nil
        view.stopLoading(); view.navigationDelegate = nil
    }
    final class Coordinator: NSObject, WKNavigationDelegate {
        var height: Binding<CGFloat>
        var observation: NSKeyValueObservation?
        var loaded = ""
        var onLoaded: (() -> Void)?
        var revision = 0
        var ready = false
        var fitting = false
        var fitAgain = false
        init(height: Binding<CGFloat>, onLoaded: (() -> Void)?) { self.height = height; self.onLoaded = onLoaded }
        func fit(_ view: WKWebView) {
            guard ready, !view.isLoading else { return }
            guard !fitting else { fitAgain = true; return }
            fitting = true
            let token = revision
            view.evaluateJavaScript(EmailHTMLView.fittingScript, in: nil, in: .defaultClient) { [weak self, weak view] result in
                guard let self, let view, token == self.revision else { return }
                self.fitting = false
                if case .success(let value) = result, let metrics = value as? [String: Any],
                   let measured = metrics["height"] as? Double, measured.isFinite, measured > 0 {
                    let next = max(100, min(30_000, CGFloat(measured)))
                    if abs(self.height.wrappedValue - next) > 0.5 { self.height.wrappedValue = next }
                    view.scrollView.isScrollEnabled = false
                } else {
                    // A failed app-side measurement must not leave wide content
                    // inaccessible. Permit panning rather than silently clipping it.
                    view.scrollView.isScrollEnabled = true
                    let next = max(100, min(30_000, view.scrollView.contentSize.height))
                    if abs(self.height.wrappedValue - next) > 0.5 { self.height.wrappedValue = next }
                }
                self.onLoaded?()
                if self.fitAgain { self.fitAgain = false; self.fit(view) }
            }
        }
        func webView(_ webView: WKWebView, decidePolicyFor navigationAction: WKNavigationAction, decisionHandler: @escaping (WKNavigationActionPolicy) -> Void) {
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
            ready = true; fit(webView)
        }
        func webView(_ webView: WKWebView, didFailProvisionalNavigation navigation: WKNavigation!, withError error: Error) {
            onLoaded?()
        }
    }
}
