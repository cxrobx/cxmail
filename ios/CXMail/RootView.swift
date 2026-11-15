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
                    MailWorkspace(store: store, chat: chat, tab: tab, settings: $settings)
                }
            }
        }
        .tint(Color.accentColor)
        .task(id: store.listKey) {
            if store.tab == .search { try? await Task.sleep(for: .milliseconds(300)); if Task.isCancelled { return } }
            await store.load()
        }
        .onChange(of: phase) { _, value in
            if value == .active { Task { await store.load() }; if store.chatPresented || store.chatDocked { chat.resume() } }
            else { chat.pause(); if let draft = store.composer { store.remember(draft) } }
        }
        .onChange(of: store.tab) { _, _ in store.selected = nil }
        .onChange(of: store.chatPresented) { _, showing in if showing { store.chatDocked = false } }
        .sheet(isPresented: $settings) { SettingsView(store: store) }
        .sheet(item: $store.composer) { draft in ComposerView(store: store, initial: draft) }
        .sheet(item: $store.review) { ref in ReviewView(store: store, ref: ref) }
        .sheet(isPresented: $store.chatPresented, onDismiss: { if !store.chatDocked { chat.pause() } }) {
            ChatView(mail: store, chat: chat, docked: false, canDock: store.chatCanDock,
                onPresentationChange: { store.chatDocked = true; store.chatPresented = false },
                onClose: { store.chatPresented = false })
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
    let chat: ChatStore
    let tab: MailTab
    @Binding var settings: Bool
    @State private var columns: NavigationSplitViewVisibility = .detailOnly
    @Environment(\.accessibilityReduceMotion) private var reduceMotion

    var body: some View {
        GeometryReader { geometry in
            let layout = geometry.mailLayout
            Group {
                if store.chatDocked && store.tab == tab {
                    DockedChatWorkspace(store: store, chat: chat) {
                        if let ref = store.selected {
                            destination(ref)
                                .toolbar { ToolbarItem(placement: .topBarLeading) {
                                    Button("Inbox", systemImage: "tray") { store.selected = nil }.accessibilityIdentifier("dockInbox")
                                } }
                        } else { MailboxView(store: store, tab: tab, split: true, settings: $settings) }
                    }
                } else if layout.split || layout.overlay {
                    NavigationSplitView(columnVisibility: $columns) {
                        MailboxView(store: store, tab: tab, split: true, settings: $settings, largeTitle: layout.overlay)
                            .padding(.trailing, max(0, layout.gap - 1))
                            .toolbar(removing: .sidebarToggle)
                            .toolbar {
                                if layout.overlay {
                                    ToolbarItem(placement: .topBarLeading) {
                                        Button("Hide inbox", systemImage: "sidebar.left") { setColumns(.detailOnly) }
                                            .accessibilityIdentifier("hideInbox")
                                    }
                                }
                            }
                            .navigationSplitViewColumnWidth(min: layout.leading + layout.gap - 1,
                                ideal: layout.leading + layout.gap - 1, max: layout.leading + layout.gap - 1)
                    } detail: {
                        NavigationStack {
                            Group {
                                if store.tab == tab, let ref = store.selected { destination(ref, split: true, sidebar: layout.overlay || columns == .detailOnly) }
                                else { ContentUnavailableView("Choose a message", systemImage: "envelope.open", description: Text(layout.overlay ? "Open Inbox to choose a message." : "Your conversation will open here.")) }
                            }
                            .toolbar {
                                if layout.overlay || columns == .detailOnly {
                                    ToolbarItem(placement: .topBarLeading) {
                                        Button("Show inbox", systemImage: "sidebar.left") { setColumns(.all) }
                                            .accessibilityIdentifier("showInbox")
                                    }
                                }
                            }
                        }
                        .accessibilityElement(children: .contain)
                        .accessibilityIdentifier("detailPane")
                        .navigationSplitViewColumnWidth(min: 300, ideal: 300, max: .infinity)
                    }
                    .modifier(MailSplitStyle(overlay: layout.overlay))
                    // Wide Duo geometry can arrive before its size-class update.
                    .environment(\.horizontalSizeClass, .regular)
                } else {
                    NavigationStack {
                        MailboxView(store: store, tab: tab, split: false, settings: $settings)
                            .navigationDestination(isPresented: Binding(get: { store.tab == tab && store.selected != nil }, set: { if !$0 { store.selected = nil } })) {
                                if let ref = store.selected { destination(ref) }
                            }
                    }
                }
            }
            .onChange(of: layout, initial: true) { _, new in
                columns = new.overlay ? .detailOnly : .all
            }
            .onChange(of: geometry.size.width >= 640, initial: true) { _, value in updateDockAvailability(value) }
            .onChange(of: store.tab) { _, _ in updateDockAvailability(geometry.size.width >= 640) }
            .onChange(of: store.selected) { _, ref in
                if layout.overlay, ref != nil { setColumns(.detailOnly) }
            }
        }
    }
    private func setColumns(_ value: NavigationSplitViewVisibility) {
        withAnimation(reduceMotion ? nil : .default) { columns = value }
    }
    private func updateDockAvailability(_ value: Bool) {
        guard store.tab == tab else { return }
        store.chatCanDock = value
        if !value && store.chatDocked { store.chatDocked = false; store.chatPresented = true }
    }
    @ViewBuilder func destination(_ ref: MessageRef, split: Bool = false, sidebar: Bool = false) -> some View {
        if tab == .drafts { ReviewView(store: store, ref: ref, embedded: true) }
        else { ReaderView(store: store, ref: ref, split: split, showsSidebar: sidebar) }
    }
}

private struct MailSplitStyle: ViewModifier {
    let overlay: Bool
    func body(content: Content) -> some View {
        if overlay { content.navigationSplitViewStyle(.prominentDetail) }
        else { content.navigationSplitViewStyle(.balanced) }
    }
}

struct MailboxView: View {
    @Bindable var store: MailStore
    let tab: MailTab
    let split: Bool
    @Binding var settings: Bool
    var largeTitle = false
    @State private var scheduled = false
    let categories = ["all", "primary", "starred", "updates", "promotions", "social"]
    var body: some View {
        List {
            if tab == .drafts, let ref = store.unavailableDraft, store.account.isEmpty || store.account == ref.account {
                Section {
                    HStack(alignment: .top, spacing: 8) {
                        VStack(alignment: .leading, spacing: 8) {
                            Label("Drafts updated", systemImage: "arrow.clockwise").font(.headline)
                            Text(MailError.draftUnavailable.localizedDescription).font(.subheadline).foregroundStyle(.secondary)
                        }.accessibilityElement(children: .combine).accessibilityIdentifier("draftUpdated")
                        Spacer(minLength: 0)
                        Button { store.unavailableDraft = nil } label: {
                            Image(systemName: "xmark").frame(width: 44, height: 44).contentShape(Rectangle())
                        }.buttonStyle(.borderless).accessibilityLabel("Dismiss draft update")
                    }.padding(.vertical, 8)
                }
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
                            store.unavailableDraft = nil
                            if tab == .drafts && !split { store.review = message.ref }
                            else { store.selected = message.ref }
                        } label: { MessageRow(message: message, sent: tab == .sent) }
                        .buttonStyle(.plain)
                        .listRowBackground(store.selected == message.ref && split ? Color.accentColor.opacity(0.14) : Color.clear)
                        .listRowInsets(.init(top: 0, leading: 16, bottom: 0, trailing: 16))
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
        .listStyle(.plain)
        .environment(\.defaultMinListRowHeight, 0)
        .scrollContentBackground(.hidden)
        .background(Color("MailCanvas"))
        .navigationTitle(tab.title)
        .navigationBarTitleDisplayMode(split && !largeTitle ? .inline : .large)
        .toolbar {
            if tab == .drafts {
                ToolbarItem(placement: .topBarTrailing) {
                    Button("Scheduled sends", systemImage: "clock") { scheduled = true }.accessibilityIdentifier("scheduledSends")
                }
            }
        }
        .sheet(isPresented: $scheduled) { ScheduledSendsView(store: store) }
        .refreshable { await store.load() }
        .safeAreaInset(edge: .top, spacing: 0) { mailboxControls }
        .safeAreaInset(edge: .bottom, spacing: 0) {
            HStack(spacing: 12) {
                Button { store.chatPresented = true } label: {
                    Image(systemName: "bubble.left.and.bubble.right").font(.title3).frame(width: 48, height: 48)
                }.mailGlass(capsule: true).accessibilityLabel("Chat").accessibilityIdentifier("openChat")
                Spacer(minLength: 0)
                Button { store.newDraft() } label: {
                    Image(systemName: "square.and.pencil").font(.title3).frame(width: 48, height: 48)
                }.mailGlass(capsule: true).accessibilityLabel("Compose").accessibilityIdentifier("compose").keyboardShortcut("n", modifiers: .command)
            }.buttonStyle(.plain).foregroundStyle(Color.accentColor).padding(.horizontal, 16).padding(.vertical, 8)
        }
        .modifier(MailSearch(enabled: tab == .search, query: $store.query))
        .sensoryFeedback(.selection, trigger: store.category)
        .sensoryFeedback(.selection, trigger: store.selected)
    }
    private var mailboxControls: some View {
        VStack(spacing: 0) {
            HStack(spacing: 8) {
                Menu {
                    Picker("Account", selection: $store.account) {
                        Text("All accounts").tag("")
                        ForEach(store.snapshot.accounts) { Text($0.email).tag($0.id) }
                    }
                } label: {
                    HStack(spacing: 8) {
                        Image(systemName: "tray.2")
                        Text(store.snapshot.accounts.first(where: { $0.id == store.account })?.email ?? "All accounts").lineLimit(1)
                        Image(systemName: "chevron.down").font(.caption2.weight(.semibold)).foregroundStyle(.secondary)
                    }.font(.subheadline.weight(.medium)).frame(minHeight: 44)
                }.accessibilityLabel("Choose account").accessibilityIdentifier("accountMenu")
                Spacer(minLength: 0)
                Button { settings = true } label: { Image(systemName: "gearshape").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Settings")
            }.padding(.leading, 16).padding(.trailing, 4)
            if tab == .inbox {
                ScrollView(.horizontal, showsIndicators: false) {
                    HStack(spacing: 20) {
                        ForEach(categories, id: \.self) { category in
                            Button { store.category = category } label: {
                                Text(category.capitalized)
                                    .font(.footnote.weight(store.category == category ? .semibold : .regular))
                                    .frame(minHeight: 44)
                                    .overlay(alignment: .bottom) {
                                        if store.category == category { Capsule().fill(Color.accentColor).frame(height: 2).padding(.bottom, 4) }
                                    }
                            }.foregroundStyle(store.category == category ? Color.accentColor : .secondary)
                                .accessibilityAddTraits(store.category == category ? .isSelected : [])
                        }
                    }
                }.contentMargins(.horizontal, 16, for: .scrollContent)
            }
        }
        .buttonStyle(.plain)
        .mailGlass()
        .padding(.horizontal, 12).padding(.top, 4).padding(.bottom, 8)
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
            Circle().fill(message.read ? Color.clear : Color.accentColor).frame(width: 6, height: 6).padding(.top, 6)
            VStack(alignment: .leading, spacing: 3) {
                HStack(alignment: .firstTextBaseline, spacing: 8) {
                    Text(sent ? "To: " + (message.recipients ?? []).joined(separator: ", ") : message.sender)
                        .font(.subheadline.weight(.semibold)).lineLimit(1).frame(maxWidth: .infinity, alignment: .leading)
                    if message.starred { Image(systemName: "star.fill").font(.caption2).foregroundStyle(.orange).accessibilityLabel("Starred") }
                    if message.attachments { Image(systemName: "paperclip").font(.caption2).foregroundStyle(.secondary).accessibilityLabel("Has attachments") }
                    if let date = message.timestamp { Text(date, format: .dateTime.month(.abbreviated).day()).font(.caption2).foregroundStyle(.secondary).monospacedDigit().fixedSize() }
                }
                Text(message.subject).font(.subheadline.weight(message.read ? .regular : .semibold)).lineLimit(2)
                if !message.snippet.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                    Text(message.snippet).font(.footnote).foregroundStyle(.secondary).lineLimit(2)
                }
            }
        }
        .foregroundStyle(.primary).padding(.vertical, 12).frame(minHeight: 72)
        .accessibilityElement(children: .combine)
        .accessibilityValue(message.read ? "Read" : "Unread")
    }
}
