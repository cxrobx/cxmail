import SwiftUI

enum AppAppearance: String, CaseIterable, Identifiable {
    case system, light, dark
    var id: String { rawValue }
    var title: String { rawValue.capitalized }
    var colorScheme: ColorScheme? {
        switch self {
        case .system: nil
        case .light: .light
        case .dark: .dark
        }
    }
    static var storageKey: String {
        #if DEBUG
        if ProcessInfo.processInfo.arguments.contains("--uitesting") { return "fixtureAppearance" }
        #endif
        return "appAppearance"
    }
}

struct SettingsView: View {
    @AppStorage(AppAppearance.storageKey) private var appearance: AppAppearance = .system
    @Bindable var store: MailStore
    @Environment(\.dismiss) private var dismiss
    @State private var clear = false
    var body: some View {
        NavigationStack {
            Form {
                Section("Appearance") {
                    Picker("Appearance", selection: $appearance) {
                        ForEach(AppAppearance.allCases) { mode in Text(mode.title).tag(mode) }
                    }
                    .pickerStyle(.segmented)
                    .accessibilityIdentifier("appearancePicker")
                }
                Section {
                    Label(store.online ? "Connected" : "Not connected", systemImage: store.online ? "checkmark.icloud" : "icloud.slash")
                    if let url = store.api.baseURL { Text(url.absoluteString).font(.footnote).textSelection(.enabled) }
                    Button("Check connection") { Task { await store.load() } }
                } header: { Text("Mac mini") } footer: { Text("Keep Tailscale connected on the phone. CXMail must be running on the mini to sync new mail. Sending and Claude also run there.") }
                Section {
                    Text("Recent mail and opened messages are cached for reading offline. Replies are saved locally as you write.")
                    Button("Clear saved mail", role: .destructive) { clear = true }
                } header: { Text("On this iPhone") } footer: { Text("Clearing mail leaves your unfinished reply intact. Cached mail and replies are excluded from device backups.") }
                NotificationSettingsSection(fixture: store.api.fixture)
                Section("About") {
                    LabeledContent("App", value: "CXMail Remote 1.0")
                    Text("Native navigation, fold-aware reading, and the same mail engine as the Mac.")
                    Text("Attachments open on the Mac.").font(.footnote).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Settings").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .confirmationDialog("Clear saved mail from this iPhone?", isPresented: $clear, titleVisibility: .visible) {
                Button("Clear saved mail", role: .destructive) { store.clearCache() }
            }
        }
        .preferredColorScheme(appearance.colorScheme)
    }
}
