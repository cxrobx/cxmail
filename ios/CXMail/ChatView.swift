import SwiftUI

struct ChatView: View {
    @Bindable var mail: MailStore
    @Bindable var chat: ChatStore
    let docked: Bool
    let canDock: Bool
    let onPresentationChange: () -> Void
    let onClose: () -> Void
    @State private var newChat = false
    @State private var following = true
    @State private var nearBottom = true
    @State private var scrolling = false
    /// The input is a UIKit text view (so it accepts pasted images); it reports focus here.
    @State private var focused = false

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
                Button("New chat") { Task { await chat.start(seed: mail.chatSeed) } }
            } message: { Text("The current chat will stop.") }
            .task { await chat.ensure(seed: mail.chatSeed) }
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 8) {
            if !chat.photos.isEmpty { PhotoStrip(photos: $chat.photos, size: 56) }
            PastingTextView(text: $chat.draft, isFocused: $focused, placeholder: "Ask about your mail…",
                            minLines: 2, maxLines: 8, identifier: "chatInput") { images in
                if chat.busy || chat.sending { return }
                if let problem = PhotoEncoder.add(images: images, to: &chat.photos, limit: 4, maxPixel: PhotoEncoder.chatPixels) { chat.error = problem }
            }
            .padding(.horizontal, 4)
            HStack(spacing: 8) {
                PhotoAddControls(photos: $chat.photos, limit: 4, maxPixel: PhotoEncoder.chatPixels) { chat.error = $0 }
                    .disabled(chat.busy || chat.sending)
                contextPicker
                Spacer(minLength: 8)
                Button {
                    if chat.busy { Task { await chat.interrupt() } }
                    else { let text = chat.draft; let context = mail.chatSeed; let photos = chat.photos; Task { _ = await chat.send(text, context: context, photos: photos) } }
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
        .background(Color.mailSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .mailGlass()
        .frame(maxWidth: 720).padding(.horizontal, 12).padding(.vertical, 8).frame(maxWidth: .infinity)
    }
    private var sendEnabled: Bool {
        !chat.sending && !chat.starting && (chat.busy || ((mail.chatContextScope == .mail || mail.selected != nil) && (!chat.draft.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !chat.photos.isEmpty)))
    }
    private var contextPicker: some View {
        Menu {
            Section("Context for next message") {
                Button { mail.chatContextScope = .mail } label: {
                    Label("Your mail", systemImage: mail.chatContextScope == .mail ? "checkmark" : "tray.2")
                }
                Button {
                    guard mail.selected != nil else { return }
                    mail.chatContextScope = .currentEmail
                } label: {
                    Label("Current email", systemImage: mail.chatContextScope == .currentEmail ? "checkmark" : "envelope")
                }.disabled(mail.selected == nil)
            }
            if let subject = mail.selected.flatMap({ ref in mail.snapshot.messages[ref.id]?.subject ?? mail.rows.first(where: { $0.ref == ref })?.subject }) {
                Text(subject)
            }
        } label: {
            HStack(spacing: 4) {
                Image(systemName: mail.chatContextScope == .mail ? "tray.2" : "envelope")
                Text(mail.chatContextScope == .mail ? "Your mail" : mail.chatContextTitle ?? "Select an email").lineLimit(1)
                Image(systemName: "chevron.down").font(.caption2)
            }.font(.caption).foregroundStyle(.secondary).frame(minHeight: 44)
        }
        .disabled(chat.sending)
        .accessibilityLabel("Chat context").accessibilityValue(mail.chatContextScope == .mail ? "Your mail" : "Current email")
        .accessibilityHint("Choose the context for your next message without clearing this conversation")
        .accessibilityIdentifier("chatContext")
    }
    private var emptyState: some View {
        VStack(alignment: .leading, spacing: 16) {
            Image(systemName: "bubble.left.and.bubble.right").font(.title).foregroundStyle(Color.accentColor)
            Text(mail.chatContextScope == .mail ? "How can I help?" : mail.selected == nil ? "Select an email" : "Ask about this email").font(.title2.weight(.semibold))
            Text(mail.chatContextScope == .currentEmail && mail.selected == nil ? "Open a message, or choose Your mail in the context menu." : "Find a message, summarize a conversation, or work on a reply.").font(.subheadline).foregroundStyle(.secondary)
            VStack(alignment: .leading, spacing: 8) {
                suggestion(mail.chatContextScope == .mail ? "Find mail I need to reply to" : "Summarize this email", symbol: "text.alignleft")
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
                HStack { Spacer(minLength: 32)
                    VStack(alignment: .trailing, spacing: 6) {
                        if !item.photos.isEmpty {
                            HStack(spacing: 6) {
                                ForEach(item.photos.indices, id: \.self) { i in
                                    Image(uiImage: item.photos[i]).resizable().scaledToFill().frame(width: 72, height: 72)
                                        .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                                        .accessibilityLabel("Sent photo").accessibilityIdentifier("chatSentPhoto")
                                }
                            }
                        }
                        if !item.text.isEmpty {
                            Text(item.text).textSelection(.enabled).padding(12).background(Color(uiColor: .tertiarySystemFill), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                        }
                    }
                }
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
                .padding(16).background(Color.mailSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
            }
        }
    }
    @ViewBuilder var permissionButtons: some View {
        Button("Allow") { Task { await chat.answer(item, allow: true, remember: false) } }.buttonStyle(.borderedProminent).controlSize(.large).accessibilityIdentifier("permissionAllow")
        if item.rememberable { Button("For this chat") { Task { await chat.answer(item, allow: true, remember: true) } }.buttonStyle(.bordered).controlSize(.large) }
        Button("Deny", role: .cancel) { Task { await chat.answer(item, allow: false, remember: false) } }.buttonStyle(.bordered).controlSize(.large).accessibilityIdentifier("permissionDeny")
    }
}
