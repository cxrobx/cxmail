import SwiftUI

// Layout and behaviour follow T3 Code's mobile thread screen (MIT,
// github.com/pingdotgg/t3code, apps/mobile/src/features/threads, 67c442a):
// assistant text without a bubble, consecutive tool calls folded into one work
// log, approvals above the composer, a floating working control with a
// scroll-to-end button, and a feed inset so nothing sits under the composer.

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
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        NavigationStack {
            ScrollViewReader { proxy in
                ScrollView {
                    LazyVStack(alignment: .leading, spacing: 0) {
                        ForEach(entries) { entry in
                            entryView(entry).id(entry.id)
                                .transition(reduceMotion ? .opacity : .opacity.combined(with: .offset(y: -6)))
                        }
                        if chat.starting { ProgressView("Connecting…").font(.footnote).padding(.vertical, 8) }
                        else if thinking { WorkStatusRow(symbol: "brain", label: "Thinking", live: true).padding(.bottom, 4) }
                        Color.clear.frame(height: 1).id("bottom")
                    }
                    .animation(reduceMotion ? nil : .easeOut(duration: 0.22), value: chat.items.count)
                    .padding(.horizontal, 16).padding(.top, 12)
                    .frame(maxWidth: 720).frame(maxWidth: .infinity)
                }
                .scrollDismissesKeyboard(.interactively)
                .defaultScrollAnchor(.bottom)
                .onScrollGeometryChange(for: Bool.self) { geometry in
                    geometry.contentOffset.y + geometry.containerSize.height >= geometry.contentSize.height - 24
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
                .safeAreaInset(edge: .bottom, spacing: 0) { bottomBar(proxy) }
            }
            .background(Color("MailCanvas"))
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
            // An approval has to be answered first; the keyboard would push the composer away.
            .onChange(of: pendingApprovals.count) { old, new in if new > old { focused = false } }
        }
    }

    // MARK: Feed

    private var entries: [ChatFeedEntry] { ChatFeedEntry.build(chat.items) }
    /// A sent turn with nothing back yet shows T3's live "Thinking" row.
    private var thinking: Bool { chat.busy && chat.items.last?.kind == .user }
    private var pendingApprovals: [ChatItem] {
        chat.items.filter { $0.kind == .permission && ($0.permissionState == "pending" || $0.permissionState == "answering") }
    }

    @ViewBuilder private func entryView(_ entry: ChatFeedEntry) -> some View {
        switch entry {
        case .work(let id, let tools):
            WorkLogGroup(tools: tools, live: chat.busy && entry.id == entries.last?.id).id(id).padding(.bottom, 8)
        case .item(let item):
            switch item.kind {
            case .user: UserMessage(item: item).padding(.top, 12).padding(.bottom, 8)
            case .assistant:
                let final = isFinalReply(item)
                VStack(alignment: .leading, spacing: 8) {
                    ChatMarkdown(text: item.text).padding(.horizontal, 3.5).accessibilityIdentifier("chatAssistantMessage")
                    if final {
                        HStack(spacing: 4) {
                            CopyTextButton(text: item.text, size: 28)
                            Text(ChatTime.label(item.created)).font(.caption.weight(.medium)).monospacedDigit().foregroundStyle(.secondary)
                        }
                    }
                }.padding(.bottom, final ? 20 : 8)
            case .notice:
                Text(item.text).font(.footnote).foregroundStyle(.secondary).padding(.bottom, 8)
            case .permission:
                // Pending approvals sit above the composer; the feed keeps the outcome.
                WorkStatusRow(symbol: item.permissionState.hasPrefix("Allowed") ? "checkmark.shield" : "hand.raised",
                              label: item.text, trailing: item.permissionState).padding(.bottom, 8)
            case .tool:
                WorkLogGroup(tools: [item], live: false).padding(.bottom, 8)
            }
        }
    }

    private func isFinalReply(_ item: ChatItem) -> Bool {
        guard !item.streaming, let index = chat.items.firstIndex(where: { $0.id == item.id }) else { return false }
        let rest = chat.items[(index + 1)...]
        if let next = rest.first(where: { $0.kind != .notice }) { return next.kind == .user }
        return !chat.busy
    }

    // MARK: Bottom bar: working control, approvals, composer

    private func bottomBar(_ proxy: ScrollViewProxy) -> some View {
        VStack(spacing: 8) {
            if chat.busy || (!following && !chat.items.isEmpty) {
                HStack(spacing: 16) {
                    if chat.busy { WorkingControl(chat: chat, waiting: !pendingApprovals.isEmpty) }
                    if !following && !chat.items.isEmpty {
                        Button {
                            following = true
                            withAnimation(reduceMotion ? nil : .easeOut(duration: 0.24)) { proxy.scrollTo("bottom", anchor: .bottom) }
                        } label: {
                            Image(systemName: "chevron.down").font(.body.weight(.semibold)).frame(width: 44, height: 44)
                        }
                        .buttonStyle(.plain).chatGlass(Circle())
                        .sensoryFeedback(.selection, trigger: following)
                        .accessibilityLabel("Scroll to end").accessibilityIdentifier("chatScrollToEnd")
                    }
                }
                .transition(.opacity)
            }
            ForEach(pendingApprovals) { item in
                ApprovalCard(chat: chat, item: item).transition(.opacity.combined(with: .offset(y: -6)))
            }
            if let error = chat.error {
                Label(error, systemImage: "exclamationmark.circle").font(.footnote).foregroundStyle(.secondary)
                    .frame(maxWidth: .infinity, alignment: .leading).padding(.horizontal, 8)
                    .accessibilityIdentifier("chatError")
            }
            composer
        }
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: chat.busy)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.18), value: following)
        .animation(reduceMotion ? nil : .easeOut(duration: 0.22), value: pendingApprovals.map(\.id))
        .frame(maxWidth: 720).padding(.horizontal, 12).padding(.top, 6).padding(.bottom, 6)
        .frame(maxWidth: .infinity)
        // The feed fades under the bar instead of showing through the composer.
        .background {
            LinearGradient(stops: [.init(color: Color("MailCanvas").opacity(0), location: 0),
                                   .init(color: Color("MailCanvas").opacity(0.6), location: 0.25),
                                   .init(color: Color("MailCanvas").opacity(0.9), location: 0.5)],
                           startPoint: .top, endPoint: .bottom)
                .ignoresSafeArea(edges: .bottom).allowsHitTesting(false)
        }
    }

    private var composer: some View {
        VStack(alignment: .leading, spacing: 4) {
            if !chat.photos.isEmpty { PhotoStrip(photos: $chat.photos, size: 56).padding(.horizontal, 6).padding(.top, 4) }
            PastingTextView(text: $chat.draft, isFocused: $focused, placeholder: placeholder,
                            minLines: 1, maxLines: 6, identifier: "chatInput") { images in
                if chat.busy || chat.sending { return }
                if let problem = PhotoEncoder.add(images: images, to: &chat.photos, limit: 4, maxPixel: PhotoEncoder.chatPixels) { chat.error = problem }
            }
            .padding(.horizontal, 10).padding(.top, 8)
            HStack(spacing: 2) {
                PhotoAddControls(photos: $chat.photos, limit: 4, maxPixel: PhotoEncoder.chatPixels, menu: true) { chat.error = $0 }
                    .disabled(chat.busy || chat.sending)
                contextPicker
                Spacer(minLength: 8)
                sendButton
            }
        }
        .padding(.horizontal, 4).padding(.bottom, 2)
        .background(Color.mailSurface.opacity(0.94), in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .chatGlass(RoundedRectangle(cornerRadius: 16, style: .continuous))
    }
    private var placeholder: String {
        pendingApprovals.isEmpty ? "Ask about your mail…" : "Answer the approval above to continue"
    }
    private var sendButton: some View {
        Button {
            if chat.busy { Task { await chat.interrupt() } }
            else { let text = chat.draft; let context = mail.chatSeed; let photos = chat.photos; Task { _ = await chat.send(text, context: context, photos: photos) } }
        } label: {
            Image(systemName: chat.busy ? "stop.fill" : "arrow.up")
                .font(.subheadline.weight(.semibold)).foregroundStyle(.white)
                .frame(width: 30, height: 30)
                .background(chat.busy ? Color.red : sendEnabled ? Color.accentColor : Color(uiColor: .tertiaryLabel), in: Circle())
                .frame(width: 44, height: 44).contentShape(Rectangle())
        }
        .disabled(!sendEnabled).buttonStyle(.plain).keyboardShortcut(.return, modifiers: .command)
        .accessibilityLabel(chat.busy ? "Stop Claude" : "Send message").accessibilityIdentifier("chatSend")
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
            }.font(.footnote.weight(.medium)).foregroundStyle(.secondary).frame(minHeight: 44).padding(.horizontal, 4)
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

// MARK: Feed model

/// Consecutive tool calls fold into one work-log entry; everything else stands alone.
enum ChatFeedEntry: Identifiable {
    case item(ChatItem)
    case work(String, [ChatItem])
    var id: String {
        switch self {
        case .item(let item): item.id
        case .work(let id, _): id
        }
    }
    static func build(_ items: [ChatItem]) -> [ChatFeedEntry] {
        var entries: [ChatFeedEntry] = []
        var tools: [ChatItem] = []
        func flush() {
            guard let first = tools.first else { return }
            entries.append(.work("work-" + first.id, tools)); tools = []
        }
        for item in items {
            switch item.kind {
            case .tool: tools.append(item)
            // Pending approvals render above the composer, not in the feed.
            case .permission where item.permissionState == "pending" || item.permissionState == "answering": continue
            default: flush(); entries.append(.item(item))
            }
        }
        flush()
        return entries
    }
}

/// How a CXMail tool reads in the work log. Unknown tools fall back to their name.
struct ToolActivity {
    let symbol: String
    let running: String
    let done: String
    /// A count phrase for the folded summary ("read 3 emails").
    let summary: (Int) -> String

    static func of(_ rawName: String) -> ToolActivity {
        let name = rawName.components(separatedBy: "__").last ?? rawName
        func times(_ verb: String) -> (Int) -> String { { $0 == 1 ? verb : "\(verb) \($0) times" } }
        func count(_ verb: String, _ one: String, _ many: String) -> (Int) -> String { { $0 == 1 ? "\(verb) \(one)" : "\(verb) \($0) \(many)" } }
        switch name {
        case "search_emails", "search_email":
            return .init(symbol: "magnifyingglass", running: "Searching mail", done: "Searched mail", summary: times("searched mail"))
        case "read_email", "read_thread", "read_email_source":
            return .init(symbol: "envelope.open", running: "Reading an email", done: "Read an email", summary: count("read", "an email", "emails"))
        case "preview_email_html":
            return .init(symbol: "eye", running: "Previewing an email", done: "Previewed an email", summary: count("previewed", "an email", "emails"))
        case "list_attachments", "download_attachment", "read_attachment_text":
            return .init(symbol: "paperclip", running: "Opening an attachment", done: "Opened an attachment", summary: count("opened", "an attachment", "attachments"))
        case "compose_draft", "edit_draft", "open_draft":
            return .init(symbol: "square.and.pencil", running: "Writing a draft", done: "Wrote a draft", summary: count("wrote", "a draft", "drafts"))
        case "send_email":
            return .init(symbol: "paperplane", running: "Sending", done: "Sent an email", summary: count("sent", "an email", "emails"))
        case "archive_email", "move_email", "delete_email", "flag_email", "bulk_delete_emails":
            return .init(symbol: "tray", running: "Updating mail", done: "Updated mail", summary: times("updated mail"))
        case "list_calendar_events", "create_calendar_event", "update_calendar_event", "send_calendar_invites":
            return .init(symbol: "calendar", running: "Checking the calendar", done: "Checked the calendar", summary: times("checked the calendar"))
        case "list_folders", "list_accounts", "list_groups":
            return .init(symbol: "folder", running: "Listing folders", done: "Listed folders", summary: times("listed folders"))
        default:
            let words = name.replacingOccurrences(of: "_", with: " ")
            return .init(symbol: "wrench", running: "Running \(words)", done: "Ran \(words)", summary: times("ran \(words)"))
        }
    }

    /// "Searched mail 2 times, read 3 emails", in first-use order.
    static func summary(_ tools: [ChatItem]) -> String {
        var order: [String] = []
        var counts: [String: (ToolActivity, Int)] = [:]
        for tool in tools {
            let activity = of(tool.text)
            if counts[activity.done] == nil { order.append(activity.done); counts[activity.done] = (activity, 0) }
            counts[activity.done]!.1 += 1
        }
        let text = order.map { counts[$0]!.0.summary(counts[$0]!.1) }.joined(separator: ", ")
        return text.prefix(1).uppercased() + text.dropFirst()
    }
}

enum ChatTime {
    static func label(_ date: Date) -> String {
        if Calendar.current.isDateInToday(date) { return date.formatted(date: .omitted, time: .shortened) }
        let sameYear = Calendar.current.isDate(date, equalTo: .now, toGranularity: .year)
        return date.formatted(sameYear ? .dateTime.month(.abbreviated).day() : .dateTime.month(.abbreviated).day().year())
    }
    static func duration(_ seconds: TimeInterval) -> String {
        let s = max(0, Int(seconds.rounded()))
        return s < 60 ? "\(s)s" : "\(s / 60)m \(s % 60)s"
    }
}

// MARK: Feed rows

struct UserMessage: View {
    let item: ChatItem
    var body: some View {
        VStack(alignment: .trailing, spacing: 4) {
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
                Text(item.text).textSelection(.enabled)
                    .padding(.horizontal, 14).padding(.vertical, 10)
                    .background(Color.chatBubble, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
                    .containerRelativeFrame(.horizontal, alignment: .trailing) { width, _ in width * 0.85 }
                    .fixedSize(horizontal: false, vertical: true)
                    .contextMenu { Button("Copy", systemImage: "doc.on.doc") { UIPasteboard.general.string = item.text } }
            }
            HStack(spacing: 4) {
                Text(ChatTime.label(item.created)).font(.caption.weight(.medium)).monospacedDigit().foregroundStyle(.secondary)
                if !item.text.isEmpty { CopyTextButton(text: item.text, size: 28) }
            }
        }
        .frame(maxWidth: .infinity, alignment: .trailing)
    }
}

/// One folded work log: a header that summarizes, and rows that open their output.
struct WorkLogGroup: View {
    let tools: [ChatItem]
    let live: Bool
    @State private var expanded = false
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        VStack(alignment: .leading, spacing: 1) {
            Button {
                withAnimation(reduceMotion ? nil : .easeOut(duration: 0.18)) { expanded.toggle() }
            } label: {
                HStack(spacing: 6) {
                    Image(systemName: live ? ToolActivity.of(tools.last?.text ?? "").symbol : "checklist")
                        .font(.system(size: 13, weight: .medium)).frame(width: 24, height: 24)
                    header.font(.subheadline).lineLimit(1)
                    Image(systemName: "chevron.down").font(.system(size: 11, weight: .semibold))
                        .rotationEffect(.degrees(expanded ? 180 : 0))
                    Spacer(minLength: 0)
                }
                .foregroundStyle(.secondary).frame(minHeight: 32).contentShape(Rectangle())
            }
            .buttonStyle(.plain)
            .sensoryFeedback(.selection, trigger: expanded)
            .accessibilityHint(expanded ? "Double tap to hide \(tools.count) tool calls." : "Double tap to show \(tools.count) tool calls.")
            .accessibilityIdentifier("chatWorkGroup")
            if expanded {
                VStack(alignment: .leading, spacing: 1) {
                    ForEach(tools) { ToolRow(tool: $0) }
                }
                .padding(.leading, 12)
                .transition(.opacity)
            }
        }
    }

    @ViewBuilder private var header: some View {
        if live, let current = tools.last {
            TimelineView(.periodic(from: .now, by: 1)) { context in
                let activity = ToolActivity.of(current.text)
                (Text(current.finished == nil ? activity.running : activity.done) + Text("  ·  " + ChatTime.duration(context.date.timeIntervalSince(tools[0].created))).monospacedDigit())
                    .shimmering()
            }
        } else {
            let failed = tools.filter(\.failed).count
            let end = tools.compactMap(\.finished).max() ?? tools.last?.created ?? .now
            Text(ToolActivity.summary(tools) + (failed > 0 ? ", \(failed) failed" : ""))
                + Text("  ·  " + ChatTime.duration(end.timeIntervalSince(tools[0].created))).monospacedDigit()
        }
    }
}

struct ToolRow: View {
    let tool: ChatItem
    @State private var open = false
    var body: some View {
        let activity = ToolActivity.of(tool.text)
        VStack(alignment: .leading, spacing: 4) {
            Button { withAnimation(.easeOut(duration: 0.14)) { open.toggle() } } label: {
                HStack(spacing: 6) {
                    Image(systemName: tool.failed ? "exclamationmark.triangle" : activity.symbol)
                        .font(.system(size: 12, weight: .medium)).frame(width: 24)
                    Text(tool.failed ? "Failed: " + activity.done.lowercased() : tool.finished == nil ? activity.running : activity.done)
                        .font(.footnote).lineLimit(1)
                    Spacer(minLength: 0)
                    if !tool.detail.isEmpty {
                        Image(systemName: "chevron.right").font(.system(size: 10, weight: .semibold)).rotationEffect(.degrees(open ? 90 : 0))
                    }
                }
                .foregroundStyle(tool.failed ? Color.orange : .secondary)
                .frame(minHeight: 28).contentShape(Rectangle())
            }.buttonStyle(.plain).disabled(tool.detail.isEmpty)
            if open {
                ScrollView {
                    Text(tool.detail).font(.caption.monospaced()).textSelection(.enabled)
                        .frame(maxWidth: .infinity, alignment: .leading).padding(10)
                }
                .frame(maxHeight: 200)
                .background(Color.mailSurface, in: RoundedRectangle(cornerRadius: 8, style: .continuous))
                .transition(.opacity)
            }
        }
    }
}

/// A single status line in the work-log style (Thinking, an approval's outcome).
struct WorkStatusRow: View {
    let symbol: String
    let label: String
    var trailing: String? = nil
    var live = false
    var body: some View {
        HStack(spacing: 6) {
            Image(systemName: symbol).font(.system(size: 13, weight: .medium)).frame(width: 24, height: 24)
            if live { Text(label).font(.subheadline).shimmering() } else { Text(label).font(.subheadline).lineLimit(1) }
            if let trailing { Text(trailing).font(.subheadline.weight(.medium)) }
            Spacer(minLength: 0)
        }
        .foregroundStyle(.secondary).frame(minHeight: 32)
    }
}

// MARK: Bottom-bar pieces

/// The floating capsule above the composer while Claude works.
struct WorkingControl: View {
    let chat: ChatStore
    let waiting: Bool
    var body: some View {
        TimelineView(.periodic(from: .now, by: 1)) { context in
            HStack(spacing: 8) {
                if waiting {
                    Image(systemName: "hand.raised").font(.footnote)
                    Text("Waiting for your approval")
                } else {
                    ProgressView().controlSize(.mini)
                    Text("Working for " + ChatTime.duration(context.date.timeIntervalSince(chat.turnStarted ?? context.date)))
                        .monospacedDigit()
                }
            }
            .font(.footnote.weight(.medium)).padding(.horizontal, 16).frame(height: 44)
        }
        .chatGlass(Capsule())
        .accessibilityElement(children: .combine)
        .accessibilityIdentifier("chatWorking")
    }
}

struct ApprovalCard: View {
    let chat: ChatStore
    let item: ChatItem
    var body: some View {
        VStack(alignment: .leading, spacing: 10) {
            Text("APPROVAL NEEDED").font(.system(size: 12, weight: .bold)).tracking(1.1).foregroundStyle(.secondary)
            Text(item.text).font(.system(size: 18, weight: .bold))
            if !item.context.isEmpty {
                Text(item.context).font(.subheadline).foregroundStyle(.secondary).lineLimit(3).textSelection(.enabled)
            }
            if !item.detail.isEmpty {
                DisclosureGroup("Details") {
                    ScrollView { Text(item.detail).font(.caption.monospaced()).textSelection(.enabled).frame(maxWidth: .infinity, alignment: .leading) }
                        .frame(maxHeight: 160)
                }.font(.footnote)
            }
            ViewThatFits(in: .horizontal) {
                HStack(spacing: 8) { buttons }.fixedSize(horizontal: true, vertical: false)
                VStack(spacing: 8) { buttons }
            }
            .controlSize(.regular)
            .disabled(item.permissionState != "pending")
        }
        .padding(14)
        .background(Color.mailSurface, in: RoundedRectangle(cornerRadius: 16, style: .continuous))
        .overlay(RoundedRectangle(cornerRadius: 16, style: .continuous).stroke(Color(uiColor: .separator).opacity(0.6), lineWidth: 1))
    }
    @ViewBuilder private var buttons: some View {
        Button { Task { await chat.answer(item, allow: true, remember: false) } } label: { Text("Allow once").lineLimit(1).frame(maxWidth: .infinity) }
            .buttonStyle(.borderedProminent).accessibilityIdentifier("permissionAllow")
        if item.rememberable {
            Button { Task { await chat.answer(item, allow: true, remember: true) } } label: { Text("Allow for this chat").lineLimit(1).frame(maxWidth: .infinity) }
                .buttonStyle(.bordered)
        }
        Button(role: .destructive) { Task { await chat.answer(item, allow: false, remember: false) } } label: { Text("Deny").lineLimit(1).frame(maxWidth: .infinity) }
            .buttonStyle(.bordered).accessibilityIdentifier("permissionDeny")
    }
}

/// Copy with a checkmark for 1.2 s and a haptic, as in T3's CopyTextButton.
struct CopyTextButton: View {
    let text: String
    var size: CGFloat = 30
    @State private var copied = false
    var body: some View {
        Button {
            UIPasteboard.general.string = text; copied = true
            Task { try? await Task.sleep(for: .milliseconds(1200)); copied = false }
        } label: {
            Image(systemName: copied ? "checkmark" : "doc.on.doc").font(.system(size: 13, weight: .medium))
                .foregroundStyle(.secondary).frame(width: size, height: size).contentShape(Rectangle())
        }
        .buttonStyle(.plain).padding(8).padding(-8)
        .sensoryFeedback(.success, trigger: copied) { _, new in new }
        .accessibilityLabel(copied ? "Copied" : "Copy message")
    }
}

// MARK: Effects

extension View {
    /// T3's working shimmer: a 72pt band sweeps over muted text, then pauses.
    func shimmering() -> some View { modifier(Shimmer()) }
    /// Liquid Glass on iOS 26, a material before it.
    @ViewBuilder func chatGlass<S: Shape>(_ shape: S) -> some View {
        if #available(iOS 26, *) { glassEffect(.regular, in: shape) }
        else { background(.regularMaterial, in: shape) }
    }
}

private struct Shimmer: ViewModifier {
    @Environment(\.accessibilityReduceMotion) private var reduceMotion
    @Environment(\.scenePhase) private var scenePhase
    func body(content: Content) -> some View {
        if reduceMotion || scenePhase != .active { content }
        else {
            content.overlay {
                TimelineView(.animation) { context in
                    let cycle = 2.8, sweep = 1.35
                    let t = context.date.timeIntervalSinceReferenceDate.truncatingRemainder(dividingBy: cycle)
                    let progress = min(1, t / sweep)
                    GeometryReader { geometry in
                        content.foregroundStyle(.primary)
                            .mask(alignment: .leading) {
                                LinearGradient(stops: [.init(color: .clear, location: 0), .init(color: .black.opacity(0.55), location: 0.35),
                                                       .init(color: .black, location: 0.5), .init(color: .black.opacity(0.55), location: 0.65),
                                                       .init(color: .clear, location: 1)], startPoint: .leading, endPoint: .trailing)
                                    .frame(width: 72)
                                    .offset(x: t < sweep ? -72 + progress * (geometry.size.width + 72) : geometry.size.width + 72)
                            }
                    }
                }
                .allowsHitTesting(false).accessibilityHidden(true)
            }
        }
    }
}
