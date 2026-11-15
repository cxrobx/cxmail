import SwiftUI

struct ChatView: View {
    @Bindable var chat: ChatStore
    let seed: MessageRef?
    let docked: Bool
    let canDock: Bool
    let onPresentationChange: () -> Void
    let onClose: () -> Void
    var contextTitle: String? = nil
    @State private var newChat = false
    @State private var following = true
    @State private var nearBottom = true
    @State private var scrolling = false
    @FocusState private var focused: Bool

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 24) {
                        ForEach(chat.items) { item in ChatItemView(chat: chat, item: item).id(item.id) }
                        if chat.starting { ProgressView("Connecting…").font(.footnote) }
                        else if chat.busy {
                            HStack(spacing: 8) {
                                if chat.items.contains(where: { $0.kind == .permission && $0.permissionState == "pending" }) {
                                    Label("Waiting for permission", systemImage: "hand.raised")
                                } else { ProgressView(); Text("Working…") }
                            }.font(.footnote).foregroundStyle(.secondary)
                                .accessibilityIdentifier("chatWorking")
                        }
                        if let error = chat.error {
                            Label(error, systemImage: "exclamationmark.circle").font(.footnote).foregroundStyle(.secondary)
                                .accessibilityIdentifier("chatError")
                        }
                        Color.clear.frame(height: 1).id("bottom")
                    }.padding(16).frame(maxWidth: 720).frame(maxWidth: .infinity)
                }
                .scrollDismissesKeyboard(.interactively)
                .defaultScrollAnchor(.bottom)
                .onScrollGeometryChange(for: Bool.self) { geometry in
                    geometry.contentOffset.y + geometry.containerSize.height >= geometry.contentSize.height - 80
                } action: { _, value in nearBottom = value; if scrolling { following = value } }
                .onScrollPhaseChange { _, phase in
                    scrolling = phase == .interacting || phase == .decelerating
                    if scrolling { following = nearBottom }
                }
                .onChange(of: chat.items.count) { _, _ in
                    if chat.items.last?.kind == .user { following = true }
                    if following { proxy.scrollTo("bottom", anchor: .bottom) }
                }
                .onChange(of: chat.items.last?.text) { _, _ in if following { proxy.scrollTo("bottom", anchor: .bottom) } }
                .overlay { if chat.items.isEmpty && !chat.starting && chat.error == nil && !focused { emptyState } }
                .overlay(alignment: .bottom) {
                    if !following && !chat.items.isEmpty {
                        Button("Latest", systemImage: "arrow.down") { following = true; proxy.scrollTo("bottom", anchor: .bottom) }
                            .font(.footnote.weight(.medium)).padding(12).mailGlass(capsule: true).padding(8)
                    }
                }
            }
            .background(Color("MailCanvas"))
            .safeAreaInset(edge: .bottom, spacing: 0) { composer }
            .navigationTitle("Claude").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) {
                    Button("Close chat", systemImage: "xmark") { focused = false; onClose() }.accessibilityIdentifier("closeChat")
                }
                ToolbarItemGroup(placement: .primaryAction) {
                    Button(docked ? "Pop out chat" : "Dock chat", systemImage: docked ? "rectangle.on.rectangle" : "rectangle.split.2x1") {
                        focused = false; onPresentationChange()
                    }.disabled(chat.starting || chat.sending || (!docked && !canDock)).accessibilityIdentifier("chatPresentation")
                    Button("New chat", systemImage: "plus.bubble") { newChat = true }.accessibilityIdentifier("newChat")
                }
            }
            .confirmationDialog("Start a new chat?", isPresented: $newChat, titleVisibility: .visible) {
                Button("New chat") { Task { await chat.start(seed: seed) } }
            } message: { Text("The current chat will stop.") }
            .task { await chat.ensure(seed: seed) }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            TextField("Ask about your mail…", text: $chat.draft, axis: .vertical)
                .font(.body).lineLimit(2...8).focused($focused)
                .accessibilityIdentifier("chatInput").padding(.horizontal, 4)
            HStack(spacing: 8) {
                Label(contextTitle ?? (seed == nil ? "Your mail" : "Email context"), systemImage: seed == nil ? "bubble.left.and.bubble.right" : "envelope")
                    .font(.caption).foregroundStyle(.secondary).lineLimit(1)
                Spacer(minLength: 8)
                Button {
                    if chat.busy { Task { await chat.interrupt() } }
                    else { let text = chat.draft; Task { _ = await chat.send(text) } }
                } label: {
                    Image(systemName: chat.busy ? "stop.fill" : "arrow.up")
                        .font(.body.weight(.semibold)).foregroundStyle(.white)
                        .frame(width: 44, height: 44)
                        .background(sendEnabled ? Color.accentColor : Color(uiColor: .tertiaryLabel), in: Circle())
                }
                .disabled(!sendEnabled).buttonStyle(.plain).keyboardShortcut(.return, modifiers: .command)
                .accessibilityLabel(chat.busy ? "Stop Claude" : "Send message").accessibilityIdentifier("chatSend")
            }
        }
        .padding(12)
        .background(Color(uiColor: .secondarySystemGroupedBackground), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .mailGlass()
        .frame(maxWidth: 720).padding(.horizontal, 12).padding(.vertical, 8).frame(maxWidth: .infinity)
    }
    private var sendEnabled: Bool {
        !chat.sending && !chat.starting && (chat.busy || !chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty)
    }
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 16) {
            Image(systemName: "bubble.left.and.bubble.right").font(.title).foregroundStyle(Color.accentColor)
            Text(seed == nil ? "How can I help?" : "Ask about this email").font(.title2.weight(.semibold))
            Text("Find a message, summarize a conversation, or work on a reply.").font(.subheadline).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 8) {
                suggestion(seed == nil ? "Find mail I need to reply to" : "Summarize this email", symbol: "text.alignleft")
                suggestion("Help me write a reply", symbol: "square.and.pencil")
            }
        }.frame(maxWidth: 400, alignment: .leading).padding(24)
    }
    private func suggestion(_ text: String, symbol: String) -> some View {
        Button { chat.draft = text; focused = true } label: {
            Label(text, systemImage: symbol).font(.subheadline).frame(minHeight: 44, alignment: .leading)
        }.buttonStyle(.plain).foregroundStyle(Color.accentColor)
    }
}

struct ChatItemView: View {
    let chat: ChatStore
    let item: ChatItem
    var body: some View {
        Group {
            switch item.kind {
            case .user:
                HStack { Spacer(minLength: 32); Text(item.text).textSelection(.enabled).padding(12).background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 16, style: .continuous)) }
                    .contextMenu { Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = item.text } }
            case .assistant:
                ChatMarkdown(text: item.text).accessibilityIdentifier("chatAssistantMessage")
                    .contextMenu { Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = item.text } }
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
