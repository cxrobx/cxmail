import SwiftUI

struct ChatView: View {
    @Bindable var chat: ChatStore
    let seed: MessageRef?
    @State private var input = ""
    @State private var sending = false
    @State private var newChat = false
    @FocusState private var focused: Bool
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 16) {
                        if chat.items.isEmpty {
                            ContentUnavailableView(seed == nil ? "Ask Claude" : "Ask about this email", systemImage: "bubble.left.and.bubble.right",
                                description: Text("Search your mail, work through a conversation, or draft a reply. Mail changes ask for your permission."))
                        }
                        ForEach(chat.items) { item in
                            ChatItemView(chat: chat, item: item).id(item.id)
                        }
                        if chat.starting { ProgressView("Starting on the mini…") }
                        if let error = chat.error { Label(error, systemImage: "exclamationmark.circle").font(.footnote).foregroundStyle(.secondary) }
                        Color.clear.frame(height: 1).id("bottom")
                    }.padding(20).frame(maxWidth: 720).frame(maxWidth: .infinity)
                }
                .scrollDismissesKeyboard(.interactively)
                .defaultScrollAnchor(.bottom)
                .onChange(of: chat.items.count) { _, _ in proxy.scrollTo("bottom", anchor: .bottom) }
            }
            .safeAreaInset(edge: .bottom) {
                HStack(alignment: .bottom, spacing: 12) {
                    TextField("Ask about your mail…", text: $input, axis: .vertical)
                        .lineLimit(1...6).focused($focused).accessibilityIdentifier("chatInput")
                        .padding(12).background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    Button {
                        if chat.busy { Task { await chat.interrupt() } }
                        else {
                            let text = input; sending = true
                            Task { if await chat.send(text) { input = "" }; sending = false }
                        }
                    } label: {
                        Image(systemName: chat.busy ? "stop.circle.fill" : "arrow.up.circle.fill").font(.title).frame(minWidth: 44, minHeight: 44)
                    }
                    .disabled(sending || chat.starting || (!chat.busy && input.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty))
                    .accessibilityLabel(chat.busy ? "Stop Claude" : "Send message").accessibilityIdentifier("chatSend")
                }.padding(16).background(.bar)
            }
            .navigationTitle("Claude").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Done") { focused = false; dismiss() } }
                ToolbarItem(placement: .primaryAction) { Button("New chat", systemImage: "square.and.pencil") { newChat = true } }
            }
            .confirmationDialog("Start a new chat?", isPresented: $newChat, titleVisibility: .visible) {
                Button("New chat") { Task { await chat.start(seed: nil) } }
            } message: { Text("The current chat on the mini will stop.") }
            .task { await chat.ensure(seed: seed) }
        }
    }
}

struct ChatItemView: View {
    let chat: ChatStore
    let item: ChatItem
    var body: some View {
        Group {
            switch item.kind {
            case .user:
                HStack { Spacer(minLength: 32); Text(item.text).textSelection(.enabled).padding(16).background(Color.accentColor.opacity(0.12), in: RoundedRectangle(cornerRadius: 16, style: .continuous)) }
            case .assistant:
                Text(item.text).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading)
            case .notice:
                Text(item.text).font(.footnote).foregroundStyle(.secondary)
            case .tool:
                DisclosureGroup {
                    Text(item.detail).font(.caption.monospaced()).textSelection(.enabled)
                } label: { Label(item.text.replacingOccurrences(of: "_", with: " "), systemImage: "wrench.and.screwdriver").font(.footnote).foregroundStyle(.secondary) }
            case .permission:
                VStack(alignment: .leading, spacing: 12) {
                    Label("Allow \(item.text)?", systemImage: "hand.raised").font(.headline)
                    if !item.context.isEmpty { Text(item.context).font(.subheadline).textSelection(.enabled) }
                    DisclosureGroup("Details") { Text(item.detail).font(.caption.monospaced()).textSelection(.enabled) }
                    if item.permissionState == "pending" {
                        ViewThatFits(in: .horizontal) {
                            HStack { permissionButtons }
                            VStack(alignment: .leading) { permissionButtons }
                        }
                    } else { Text(item.permissionState).font(.footnote).foregroundStyle(.secondary) }
                }
                .padding(16).background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            }
        }
    }
    @ViewBuilder var permissionButtons: some View {
        Button("Allow") { Task { await chat.answer(item, allow: true, remember: false) } }.buttonStyle(.borderedProminent).controlSize(.large).accessibilityIdentifier("permissionAllow")
        if item.rememberable { Button("For this chat") { Task { await chat.answer(item, allow: true, remember: true) } }.buttonStyle(.bordered).controlSize(.large) }
        Button("Deny", role: .cancel) { Task { await chat.answer(item, allow: false, remember: false) } }.buttonStyle(.bordered).controlSize(.large).accessibilityIdentifier("permissionDeny")
    }
}
