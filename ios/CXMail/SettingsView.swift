import SwiftUI

struct SettingsView: View {
    @Bindable var store: MailStore
    @Environment(\.dismiss) private var dismiss
    @State private var clear = false
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Label(store.online ? "Connected" : "Not connected", systemImage: store.online ? "checkmark.icloud" : "icloud.slash")
                    if let url = store.api.baseURL { Text(url.absoluteString).font(.footnote).textSelection(.enabled) }
                    Button("Check connection") { Task { await store.load() } }
                } header: { Text("Mac mini") } footer: { Text("Keep Tailscale connected on the phone. CXMail must be running on the mini to sync new mail. Sending and Claude also run there.") }
                Section {
                    Text("Recent mail and opened messages are cached for reading offline. Replies are saved locally as you write.")
                    Button("Clear saved mail", role: .destructive) { clear = true }
                } header: { Text("On this iPhone") } footer: { Text("Clearing mail leaves your unfinished reply intact. Cached mail and replies are excluded from device backups.") }
                Section("About") {
                    LabeledContent("App", value: "CXMail Remote 1.0")
                    Text("Native navigation, fold-aware reading, and the same mail engine as the Mac.")
                    Text("Attachments open on the Mac. Push notifications are not enabled in this version.").font(.footnote).foregroundStyle(.secondary)
                }
            }
            .navigationTitle("Settings").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() } } }
            .confirmationDialog("Clear saved mail from this iPhone?", isPresented: $clear, titleVisibility: .visible) {
                Button("Clear saved mail", role: .destructive) { store.clearCache() }
            }
        }
    }
}
