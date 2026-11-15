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
            cache.remove("mail.json"); cache.remove("draft.json")
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
            if store.api.fixture && ProcessInfo.processInfo.arguments.contains("--duo-wide") {
                RootView(store: store).frame(width: 867, height: 635).frame(maxWidth: .infinity, maxHeight: .infinity)
            } else { RootView(store: store) }
            #else
            RootView(store: store)
            #endif
        }
    }
}
