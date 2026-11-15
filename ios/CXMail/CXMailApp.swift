import SwiftUI

@main
struct CXMailApp: App {
    @State private var store: MailStore
    init() {
        var fixture = false
        #if DEBUG
        fixture = ProcessInfo.processInfo.arguments.contains("--uitesting")
        #endif
        let url = URL(string: Bundle.main.object(forInfoDictionaryKey: "CXMailServerURL") as? String ?? "")
        let cache = fixture ? MailCache(directory: FileManager.default.temporaryDirectory.appendingPathComponent("cxmail-ui-fixture")) : MailCache()
        #if DEBUG
        if fixture && ProcessInfo.processInfo.arguments.contains("--reset") {
            cache.remove("mail.json"); cache.remove("mail-v2.json"); cache.remove("draft.json")
        }
        #endif
        let model = MailStore(api: APIClient(baseURL: url, fixture: fixture), cache: cache)
        #if DEBUG
        if fixture && ProcessInfo.processInfo.arguments.contains("--show-reader") { model.selected = .init(account: "work", folder: "INBOX", uid: 10) }
        #endif
        _store = State(initialValue: model)
    }
    var body: some Scene {
        WindowGroup {
            #if DEBUG
            if store.api.fixture && (ProcessInfo.processInfo.arguments.contains("--duo-wide") || ProcessInfo.processInfo.arguments.contains("--duo-flat") || ProcessInfo.processInfo.arguments.contains("--duo-tall")) {
                let tall = ProcessInfo.processInfo.arguments.contains("--duo-tall")
                DuoFixtureViewport(store: store, tall: tall).frame(width: tall ? 669 : 951, height: tall ? 951 : 669).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else { RootView(store: store) }
            #else
            RootView(store: store)
            #endif
        }
    }
}

#if DEBUG
/// Include the asymmetric status safe area, not just the usable rectangle:
/// nested navigation controllers must not reserve it again inside each pane.
private struct DuoFixtureViewport: UIViewControllerRepresentable {
    let store: MailStore
    let tall: Bool
    func makeUIViewController(context: Context) -> UIHostingController<RootView> {
        let host = UIHostingController(rootView: RootView(store: store))
        host.additionalSafeAreaInsets = UIEdgeInsets(top: tall ? 82 : 0, left: 0, bottom: 34, right: tall ? 0 : 84)
        return host
    }
    func updateUIViewController(_ host: UIHostingController<RootView>, context: Context) {}
}
#endif
