import SwiftUI

struct RootView: View {
    @Bindable var store: MailStore
    @State private var settings = false
    @State private var chat: ChatStore
    @Environment(\.scenePhase) private var phase

    init(store: MailStore) { self.store = store; _chat = State(initialValue: ChatStore(api: store.api)) }
    var body: some View {
        TabView(selection: $store.tab) {
            ForEach(MailTab.allCases) { tab in
                Tab(tab.title, systemImage: tab.symbol, value: tab) {
                    MailWorkspace(store: store, tab: tab, settings: $settings)
                }
            }
        }
        .tint(Color.accentColor)
        .task(id: store.listKey) {
            if store.tab == .search { try? await Task.sleep(for: .milliseconds(300)); if Task.isCancelled { return } }
            await store.load()
        }
        .onChange(of: phase) { _, value in
            if value == .active { Task { await store.load() }; if store.chatPresented { chat.resume() } }
            else { chat.pause(); if let draft = store.composer { store.remember(draft) } }
        }
        .onChange(of: store.tab) { _, _ in store.selected = nil }
        .sheet(isPresented: $settings) { SettingsView(store: store) }
        .sheet(item: $store.composer) { draft in ComposerView(store: store, initial: draft) }
        .sheet(item: $store.review) { ref in ReviewView(store: store, ref: ref) }
        .sheet(isPresented: $store.chatPresented, onDismiss: { chat.pause() }) {
            ChatView(chat: chat, seed: store.chatSeed)
        }
        .onOpenURL { url in
            guard url.scheme == "cxmail" else { return }
            if url.host == "compose" { store.newDraft() }
            if let tab = MailTab(rawValue: url.host ?? "") { store.tab = tab }
        }
        .alert("CXMail", isPresented: Binding(get: { store.notice != nil }, set: { if !$0 { store.notice = nil } })) {
            Button("OK", role: .cancel) { store.notice = nil }
        } message: { Text(store.notice ?? "") }
    }
}

struct MailWorkspace: View {
    @Bindable var store: MailStore
    let tab: MailTab
    @Binding var settings: Bool
    var body: some View {
        GeometryReader { geometry in
            let layout = geometry.mailLayout
            HStack(spacing: 0) {
                NavigationStack {
                    MailboxView(store: store, tab: tab, split: layout.split, settings: $settings)
                        .navigationDestination(isPresented: Binding(get: { !layout.split && store.tab == tab && store.selected != nil }, set: { if !$0 && !layout.split { store.selected = nil } })) {
                            if let ref = store.selected { destination(ref) }
                        }
                }
                .frame(width: layout.split ? layout.leading : nil)
                if layout.split {
                    Rectangle().fill(Color(uiColor: .separator)).frame(width: 1)
                    Color(uiColor: .systemGroupedBackground).frame(width: max(0, layout.gap - 1))
                    NavigationStack {
                        if let ref = store.selected { destination(ref) }
                        else { ContentUnavailableView("Choose a message", systemImage: "envelope.open", description: Text("Your conversation will open here.")) }
                    }
                    .frame(maxWidth: .infinity)
                    .accessibilityElement(children: .contain)
                    .accessibilityIdentifier("detailPane")
                }
            }
        }
    }
    @ViewBuilder func destination(_ ref: MessageRef) -> some View {
        if tab == .drafts { ReviewView(store: store, ref: ref, embedded: true) }
        else { ReaderView(store: store, ref: ref) }
    }
}

struct MailboxView: View {
    @Bindable var store: MailStore
    let tab: MailTab
    let split: Bool
    @Binding var settings: Bool
    let categories = ["primary", "starred", "updates", "promotions", "social", "all"]
    var body: some View {
        List {
            Section {
                HStack {
                    Menu {
                        Picker("Account", selection: $store.account) {
                            Text("All accounts").tag("")
                            ForEach(store.snapshot.accounts) { Text($0.email).tag($0.id) }
                        }
                    } label: {
                        Label(store.snapshot.accounts.first(where: { $0.id == store.account })?.email ?? "All accounts", systemImage: "person.crop.circle")
                            .lineLimit(1).frame(minHeight: 44)
                    }.accessibilityLabel("Choose account").accessibilityIdentifier("accountMenu")
                    Spacer(minLength: 8)
                    Button { settings = true } label: { Image(systemName: "gearshape").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Settings")
                }.buttonStyle(.plain)
            }
            if let error = store.error {
                Section {
                    VStack(alignment: .leading, spacing: 8) {
                        Label("Connection needs attention", systemImage: "wifi.exclamationmark").font(.headline)
                        Text(error).font(.subheadline).foregroundStyle(.secondary)
                        Button("Try again") { Task { await store.load() } }.accessibilityIdentifier("retryConnection")
                    }.padding(.vertical, 8)
                }
            }
            if tab == .inbox {
                Section {
                    ScrollView(.horizontal, showsIndicators: false) {
                        HStack(spacing: 8) {
                            ForEach(categories, id: \.self) { category in
                                Button { store.category = category } label: {
                                    Text(category == "all" ? "All mail" : category.capitalized)
                                        .font(.subheadline.weight(store.category == category ? .semibold : .regular))
                                        .padding(.horizontal, 12).frame(minHeight: 44)
                                        .background(store.category == category ? Color.accentColor.opacity(0.12) : Color(uiColor: .tertiarySystemFill), in: Capsule())
                                }.buttonStyle(.plain).foregroundStyle(store.category == category ? Color.accentColor : .secondary)
                                    .accessibilityAddTraits(store.category == category ? .isSelected : [])
                            }
                        }
                    }
                    .listRowInsets(.init(top: 0, leading: 0, bottom: 0, trailing: 0))
                    .listRowBackground(Color.clear)
                }
            }
            if let draft = store.recoveredDraft {
                Section {
                    Button { store.composer = draft } label: {
                        Label { VStack(alignment: .leading) { Text("Continue your reply").font(.headline); Text(draft.subject.isEmpty ? "Saved on this iPhone" : draft.subject).font(.subheadline).foregroundStyle(.secondary) } } icon: { Image(systemName: "square.and.pencil") }
                    }.accessibilityIdentifier("recoverDraft")
                }
            }
            Section {
                if store.loading && store.rows.isEmpty {
                    ForEach(0..<5) { _ in MessageRow(message: Self.placeholder, sent: false).redacted(reason: .placeholder).accessibilityHidden(true) }
                } else if store.rows.isEmpty {
                    ContentUnavailableView(tab == .search ? "Search your mail" : "You’re all caught up", systemImage: tab.symbol,
                        description: Text(tab == .search ? "Find a sender, subject, or phrase across your accounts." : "Pull to refresh or choose another mailbox."))
                        .listRowBackground(Color.clear)
                } else {
                    ForEach(store.rows) { message in
                        Button {
                            if tab == .drafts && !split { store.review = message.ref }
                            else { store.selected = message.ref }
                        } label: { MessageRow(message: message, sent: tab == .sent) }
                        .buttonStyle(.plain)
                        .listRowBackground(store.selected == message.ref && split ? Color.accentColor.opacity(0.10) : Color(uiColor: .secondarySystemGroupedBackground))
                        .accessibilityIdentifier("message-\(message.uid)")
                        .contextMenu { MailActions(store: store, message: message) }
                        .swipeActions(edge: .trailing, allowsFullSwipe: false) {
                            if tab != .drafts {
                                Button("Archive", systemImage: "archivebox") { Task { await store.action("archive", message) } }.tint(.accentColor)
                                Button(message.starred ? "Unstar" : "Star", systemImage: message.starred ? "star.slash" : "star") { Task { await store.action(message.starred ? "unstar" : "star", message) } }.tint(.orange)
                            }
                        }
                        .swipeActions(edge: .leading, allowsFullSwipe: true) {
                            if tab != .drafts {
                                Button(message.read ? "Unread" : "Read", systemImage: message.read ? "envelope.badge" : "envelope.open") { Task { await store.action(message.read ? "unread" : "read", message) } }.tint(.accentColor)
                            }
                        }
                        .disabled(store.pending.contains(message.id))
                    }
                    if store.hasMore {
                        Button { Task { await store.load(more: true) } } label: { HStack { Spacer(); if store.loadingMore { ProgressView() } else { Text("Load more") }; Spacer() } }
                            .disabled(store.loadingMore)
                    }
                }
            } footer: {
                if let date = store.snapshot.savedAt {
                    HStack(spacing: 4) {
                        Image(systemName: store.online ? "checkmark.icloud" : "icloud.slash")
                        Text(store.online ? "Updated" : "Saved")
                        Text(date, style: .relative).monospacedDigit()
                    }.frame(maxWidth: .infinity).padding(.top, 8)
                }
            }
        }
        .listStyle(.insetGrouped)
        .navigationTitle(tab.title)
        .navigationBarTitleDisplayMode(.large)
        .refreshable { await store.load() }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            HStack(spacing: 12) {
                Button { store.chatSeed = nil; store.chatPresented = true } label: {
                    Label("Ask Claude", systemImage: "bubble.left.and.bubble.right").frame(maxWidth: .infinity, minHeight: 44)
                }.accessibilityIdentifier("openChat").buttonStyle(.bordered)
                Button { store.newDraft() } label: {
                    Label("Compose", systemImage: "square.and.pencil").frame(maxWidth: .infinity, minHeight: 44)
                }.accessibilityIdentifier("compose").buttonStyle(.borderedProminent).keyboardShortcut("n", modifiers: .command)
            }.padding(.horizontal, 16).padding(.vertical, 8).background(.bar)
        }
        .modifier(MailSearch(enabled: tab == .search, query: $store.query))
        .sensoryFeedback(.selection, trigger: store.category)
        .sensoryFeedback(.selection, trigger: store.selected)
    }
    static let placeholder = MailMessage(account: "", folder: "", uid: 0, subject: "A note about next week", fromName: "Your correspondent", date: "", snippet: "A preview of your message will appear here.", read: false, starred: false, attachments: false)
}

struct MailSearch: ViewModifier {
    var enabled: Bool
    @Binding var query: String
    func body(content: Content) -> some View {
        if enabled { content.searchable(text: $query, placement: .navigationBarDrawer(displayMode: .always), prompt: "Search all mail").autocorrectionDisabled() }
        else { content }
    }
}

struct MessageRow: View {
    let message: MailMessage
    let sent: Bool
    var body: some View {
        HStack(alignment: .top, spacing: 8) {
            Circle().fill(message.read ? Color.clear : Color.accentColor).frame(width: 8, height: 8).padding(.top, 7)
            VStack(alignment: .leading, spacing: 4) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(sent ? "To: " + (message.recipients ?? []).joined(separator: ", ") : message.sender)
                        .font(.headline).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    if let date = message.timestamp { Text(date, format: .dateTime.month(.abbreviated).day()).font(.caption).foregroundStyle(.secondary).monospacedDigit().fixedSize() }
                }
                Text(message.subject).font(.subheadline.weight(message.read ? .regular : .semibold)).lineLimit(2)
                Text(message.snippet).font(.subheadline).foregroundStyle(.secondary).lineLimit(2)
                if message.starred || message.attachments {
                    HStack(spacing: 8) {
                        if message.starred { Image(systemName: "star.fill").foregroundStyle(.orange).accessibilityLabel("Starred") }
                        if message.attachments { Image(systemName: "paperclip").accessibilityLabel("Has attachments") }
                    }.font(.caption).foregroundStyle(.secondary)
                }
            }
        }
        .foregroundStyle(.primary).padding(.vertical, 8).frame(minHeight: 80)
        .accessibilityElement(children: .combine)
        .accessibilityValue(message.read ? "Read" : "Unread")
    }
}
