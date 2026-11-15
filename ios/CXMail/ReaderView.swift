import SwiftUI

struct ReaderView: View {
    @Bindable var store: MailStore
    let ref: MessageRef
    @State private var message: MailMessage?
    @State private var html: String?
    @State private var height: CGFloat = 240
    @State private var images = false
    @State private var loading = true
    @State private var error: String?
    @State private var showThread = false
    @State private var move = false
    @State private var trash = false
    @State private var details = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        Group {
            if let message {
                ScrollView {
                    VStack(alignment: .leading, spacing: 16) {
                        VStack(alignment: .leading, spacing: 12) {
                            Text(message.subject).font(.title2.bold()).textSelection(.enabled).accessibilityIdentifier("readerSubject")
                            HStack(alignment: .top, spacing: 12) {
                                Image(systemName: "person.crop.circle.fill").font(.largeTitle).foregroundStyle(.secondary)
                                VStack(alignment: .leading, spacing: 4) {
                                    Text(message.sender).font(.headline).textSelection(.enabled)
                                    if let date = message.timestamp { Text(date, format: .dateTime.month(.wide).day().hour().minute()).font(.caption).foregroundStyle(.secondary).monospacedDigit() }
                                }
                                Spacer(minLength: 0)
                                Button { details.toggle() } label: { Image(systemName: details ? "chevron.up.circle" : "chevron.down.circle").frame(minWidth: 44, minHeight: 44) }.accessibilityLabel("Recipient details")
                            }
                            if details {
                                AddressLine(title: "From", addresses: [.init(name: message.fromName, email: message.fromEmail ?? "")])
                                AddressLine(title: "To", addresses: message.to ?? [])
                                AddressLine(title: "Cc", addresses: message.cc ?? [])
                            }
                            if let error { Label(error, systemImage: "wifi.exclamationmark").font(.footnote).foregroundStyle(.secondary) }
                            if !(message.thread ?? []).isEmpty {
                                Button { showThread = true } label: { Label("\((message.thread ?? []).count + 1) messages in conversation", systemImage: "text.bubble").font(.subheadline).frame(minHeight: 44) }
                                    .accessibilityIdentifier("showConversation")
                            }
                        }
                        .padding(.horizontal, 20)
                        if message.hasHtml == true {
                            HStack {
                                Label(images ? "Remote images enabled" : "Remote images blocked", systemImage: images ? "photo" : "eye.slash")
                                    .font(.caption).foregroundStyle(.secondary)
                                Spacer()
                                if !images { Button("Show images") { Task { await loadImages() } }.font(.caption).frame(minHeight: 44) }
                            }.padding(.horizontal, 20)
                            if let html {
                                EmailHTMLView(html: html, images: images, height: $height).frame(height: height)
                            } else if loading { ProgressView("Loading message…").frame(maxWidth: .infinity).padding() }
                            else { Button("Try loading the message again") { Task { await load() } }.padding(20) }
                        } else {
                            Text(message.plainText ?? "The message body isn’t available yet. Refresh after CXMail syncs on the mini.")
                                .font(.body).textSelection(.enabled).padding(.horizontal, 20)
                        }
                        if let names = message.attachmentNames, !names.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                ForEach(names, id: \.self) { Label($0, systemImage: "paperclip").font(.subheadline).textSelection(.enabled) }
                                Text("Open attachments from CXMail on the Mac.").font(.caption).foregroundStyle(.secondary)
                            }.padding(20).frame(maxWidth: .infinity, alignment: .leading).background(Color(uiColor: .secondarySystemGroupedBackground))
                        }
                    }.padding(.vertical, 16)
                }
                .background(Color(uiColor: .systemBackground))
                .refreshable { await load() }
                .safeAreaInset(edge: .bottom, spacing: 0) {
                    HStack(spacing: 0) {
                        Button { Task { if await store.action("archive", message) { dismiss() } } } label: {
                            Label("Archive", systemImage: "archivebox").frame(maxWidth: .infinity, minHeight: 44)
                        }.accessibilityIdentifier("readerArchive")
                        Button {
                            Task { if await store.action(message.starred ? "unstar" : "star", message) { self.message?.starred.toggle() } }
                        } label: {
                            Label(message.starred ? "Unstar" : "Star", systemImage: message.starred ? "star.fill" : "star").frame(maxWidth: .infinity, minHeight: 44)
                        }.accessibilityIdentifier("readerStar")
                        Button { store.chatSeed = ref; store.chatPresented = true } label: {
                            Label("Ask Claude", systemImage: "bubble.left.and.bubble.right").frame(maxWidth: .infinity, minHeight: 44)
                        }.accessibilityIdentifier("readerChat")
                        Button { store.newDraft(reply: message) } label: {
                            Label("Reply", systemImage: "arrowshape.turn.up.left").frame(maxWidth: .infinity, minHeight: 44)
                        }.accessibilityIdentifier("readerReply")
                        Menu {
                            Button("Reply all", systemImage: "arrowshape.turn.up.left.2") { store.newDraft(reply: message, all: true) }
                            Button(message.read ? "Mark unread" : "Mark read", systemImage: "envelope.badge") { Task { if await store.action(message.read ? "unread" : "read", message) { self.message?.read.toggle() } } }
                            Button("Move to…", systemImage: "folder") { move = true }
                            Button("Open in Claude", systemImage: "arrow.up.forward.app") { Task { await openThread() } }
                            Button("Move to Trash", systemImage: "trash", role: .destructive) { trash = true }
                        } label: { Image(systemName: "ellipsis.circle").frame(maxWidth: .infinity, minHeight: 44) }.accessibilityLabel("Message actions")
                    }.labelStyle(.iconOnly).font(.title3).padding(.horizontal, 16).padding(.vertical, 8).background(.bar)
                }
                .disabled(store.pending.contains(message.id))
                .sheet(isPresented: $move) { MoveView(store: store, message: message) }
                .sheet(isPresented: $showThread) {
                    NavigationStack {
                        List(message.thread ?? []) { member in
                            Button { showThread = false; store.selected = member.ref } label: { MessageRow(message: member, sent: false) }.buttonStyle(.plain)
                        }
                        .navigationTitle("Conversation").navigationBarTitleDisplayMode(.inline)
                        .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { showThread = false } } }
                    }.presentationDetents([.medium, .large])
                }
                .confirmationDialog("Move this message to Trash?", isPresented: $trash, titleVisibility: .visible) {
                    Button("Move to Trash", role: .destructive) { Task { if await store.action("trash", message) { dismiss() } } }
                } message: { Text("You can recover it from Trash on the Mac.") }
            } else if loading { ProgressView("Loading message…") }
            else {
                ContentUnavailableView { Label("Message unavailable", systemImage: "envelope.badge") } description: { Text(error ?? "Try refreshing the inbox.") } actions: { Button("Try again") { Task { await load() } } }
            }
        }
        .navigationTitle("Message").navigationBarTitleDisplayMode(.inline)
        .task(id: ref) { await load() }
    }
    private func load() async {
        message = store.snapshot.messages[ref.id]; html = store.snapshot.html[ref.id]
        images = false; error = nil; loading = true
        defer { loading = false }
        do {
            let loaded = try await store.message(ref)
            guard !Task.isCancelled else { return }
            message = loaded
            if loaded.hasHtml == true { html = try await store.body(ref) }
            guard !Task.isCancelled else { return }
            if !loaded.read, await store.action("read", loaded) { message?.read = true }
        } catch { if !Task.isCancelled { self.error = store.friendly(error) } }
    }
    private func loadImages() async {
        do { html = try await store.body(ref, images: true); images = true }
        catch { self.error = store.friendly(error) }
    }
    private func openThread() async {
        do {
            _ = try await store.api.data("/api/claude-thread", body: ref.payload)
            store.notice = "Opened a Claude thread in T3 on the mini."
        } catch { store.error = store.friendly(error) }
    }
}

struct AddressLine: View {
    let title: String
    let addresses: [MailAddress]
    var body: some View {
        if !addresses.isEmpty {
            VStack(alignment: .leading, spacing: 4) {
                Text(title).font(.caption.weight(.semibold)).foregroundStyle(.secondary)
                ForEach(Array(addresses.enumerated()), id: \.offset) { _, address in
                    Text(address.name.map { "\($0) <\(address.email)>" } ?? address.email).font(.subheadline).textSelection(.enabled)
                }
            }.frame(maxWidth: .infinity, alignment: .leading)
        }
    }
}

struct MailActions: View {
    let store: MailStore
    let message: MailMessage
    var body: some View {
        if message.draft == true { Button("Review before sending", systemImage: "paperplane") { store.review = message.ref } }
        else {
            Button("Reply", systemImage: "arrowshape.turn.up.left") { Task { await reply(false) } }
            Button("Reply all", systemImage: "arrowshape.turn.up.left.2") { Task { await reply(true) } }
            Button("Ask Claude", systemImage: "bubble.left.and.bubble.right") { store.chatSeed = message.ref; store.chatPresented = true }
            Button("Archive", systemImage: "archivebox") { Task { await store.action("archive", message) } }
            Button(message.starred ? "Unstar" : "Star", systemImage: "star") { Task { await store.action(message.starred ? "unstar" : "star", message) } }
            Button(message.read ? "Mark unread" : "Mark read", systemImage: "envelope.badge") { Task { await store.action(message.read ? "unread" : "read", message) } }
        }
    }
    private func reply(_ all: Bool) async {
        do { store.newDraft(reply: try await store.message(message.ref), all: all) }
        catch { store.error = store.friendly(error) }
    }
}

struct MoveView: View {
    let store: MailStore
    let message: MailMessage
    @State private var folders: [MailFolder] = []
    @State private var error: String?
    @State private var busy = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                if let error { Text(error).foregroundStyle(.red) }
                if folders.isEmpty && error == nil { ProgressView("Loading folders…") }
                ForEach(folders.filter { $0.name != message.folder && $0.type != "drafts" }) { folder in
                    Button(folder.label, systemImage: "folder") {
                        busy = true
                        Task { if await store.action("move", message, folder: folder.name) { dismiss() }; busy = false }
                    }.disabled(busy)
                }
            }
            .navigationTitle("Move message").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() } } }
            .task {
                do { folders = try await store.api.get("/api/folders", query: [.init(name: "account", value: message.account)]) }
                catch { self.error = store.friendly(error) }
            }
        }.presentationDetents([.medium, .large])
    }
}
