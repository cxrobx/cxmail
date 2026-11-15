import SwiftUI

struct ReaderView: View {
    @Bindable var store: MailStore
    let ref: MessageRef
    var split = false
    var showsSidebar = false
    @State private var webLoading = true
    @State private var message: MailMessage?
    @State private var html: String?
    @State private var height: CGFloat = 240
    @State private var images = false
    @State private var imageGeneration = 0
    @State private var updatingImageTrust = false
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
                    VStack(alignment: .leading, spacing: 12) {
                        messageHeader(message)
                        if let bodyError = message.bodyError {
                            bodyFailure(bodyError)
                        } else if message.hasHtml == true {
                            if message.imagePolicy?.hasRemoteImages != false {
                                HStack(spacing: 8) {
                                    if !images {
                                        Text("Images blocked").font(.caption).foregroundStyle(.secondary)
                                        Button("Show images") { Task { await loadImages() } }
                                            .font(.caption.weight(.medium)).frame(minHeight: 44).accessibilityIdentifier("showImages")
                                    }
                                    Spacer(minLength: 8)
                                    Menu {
                                        if message.imagePolicy?.senderTrusted == true {
                                            Button("Stop trusting this sender", role: .destructive) { Task { await setImageTrust(false) } }
                                        } else {
                                            Button("Always trust this sender") { Task { await setImageTrust(true) } }
                                                .disabled(message.imagePolicy?.authentication.status != "passed" || message.imagePolicy?.suspicious != false)
                                        }
                                    } label: { Image(systemName: "photo").frame(minWidth: 44, minHeight: 44) }
                                    .accessibilityLabel("Sender image settings").accessibilityIdentifier("senderImageSettings").disabled(updatingImageTrust)
                                }.padding(.horizontal, 20)
                                if !images, message.imagePolicy?.senderTrusted == true, let reason = message.imagePolicy?.blockedReason {
                                    Text(reason).font(.caption).foregroundStyle(.secondary).padding(.horizontal, 20)
                                }
                            }
                            if let html {
                                EmailHTMLView(html: html, images: images, height: $height, onLoaded: { webLoading = false })
                                    .frame(height: height)
                                    .overlay { if webLoading { ProgressView().padding(16).background(.regularMaterial, in: Capsule()) } }
                                    .clipShape(RoundedRectangle(cornerRadius: 12, style: .continuous))
                                    .padding(.horizontal, 12)
                            } else if loading {
                                ProgressView("Loading message…").frame(maxWidth: .infinity).padding(32)
                            } else { bodyFailure(error ?? "We couldn’t load this message.") }
                        } else if let text = message.plainText, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty {
                            Text(text).font(.body).textSelection(.enabled).padding(.horizontal, 20).padding(.vertical, 8)
                        } else if loading {
                            ProgressView("Loading message…").frame(maxWidth: .infinity).padding(32)
                        } else {
                            ContentUnavailableView("Empty message", systemImage: "envelope.open", description: Text("This email has no message content."))
                        }
                        if let names = message.attachmentNames, !names.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                ForEach(names, id: \.self) { Label($0, systemImage: "paperclip").font(.footnote).textSelection(.enabled) }
                                Text("Open attachments on the Mac").font(.caption2).foregroundStyle(.secondary)
                            }.padding(16).frame(maxWidth: .infinity, alignment: .leading)
                        }
                    }.padding(.top, 16).padding(.bottom, 8)
                }
                .background(Color("MailCanvas"))
                .refreshable { await load() }
                .safeAreaInset(edge: .bottom, spacing: 0) { actionBar(message) }
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
        .navigationTitle(split ? "" : "Message").navigationBarTitleDisplayMode(.inline)
        .toolbar(split && !showsSidebar ? .hidden : .visible, for: .navigationBar)
        .task(id: ref) { await load() }
    }

    private func messageHeader(_ message: MailMessage) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Text(message.subject).font(.title3.weight(.semibold)).textSelection(.enabled).accessibilityIdentifier("readerSubject")
            HStack(alignment: .firstTextBaseline, spacing: 12) {
                HStack(spacing: 4) {
                    Text(message.sender).font(.subheadline.weight(.semibold)).textSelection(.enabled)
                    if let auth = message.imagePolicy?.authentication {
                        Button { details.toggle() } label: { Image(systemName: auth.symbol).font(.caption).foregroundStyle(Color.secondary).frame(minWidth: 24, minHeight: 44) }
                            .accessibilityLabel(auth.label).accessibilityIdentifier("senderAuthentication")
                    }
                }
                Spacer(minLength: 8)
                if let date = message.timestamp { Text(date, format: .dateTime.month(.abbreviated).day().hour().minute()).font(.caption2).foregroundStyle(.secondary).monospacedDigit() }
            }
            HStack(spacing: 12) {
                Button { details.toggle() } label: {
                    HStack(spacing: 4) {
                        Text("To " + recipientSummary(message)).lineLimit(1)
                        Image(systemName: details ? "chevron.up" : "chevron.down").font(.caption2)
                    }.font(.caption).foregroundStyle(.secondary).frame(minHeight: 44)
                }.accessibilityLabel("Recipient details")
                Spacer(minLength: 0)
                if !(message.thread ?? []).isEmpty {
                    Button { showThread = true } label: {
                        Label("\((message.thread ?? []).count + 1)", systemImage: "text.bubble").font(.caption).frame(minHeight: 44)
                    }.accessibilityLabel("\((message.thread ?? []).count + 1) messages in conversation").accessibilityIdentifier("showConversation")
                }
            }
            if details {
                if let auth = message.imagePolicy?.authentication {
                    Text(auth.label + (auth.domain.map { ": " + $0 } ?? "")).font(.caption.weight(.medium))
                    Text(auth.explanation).font(.caption).foregroundStyle(.secondary)
                }
                AddressLine(title: "From", addresses: [.init(name: message.fromName, email: message.fromEmail ?? "")])
                AddressLine(title: "To", addresses: message.to ?? [])
                AddressLine(title: "Cc", addresses: message.cc ?? [])
            }
            if let error { Label(error, systemImage: "wifi.exclamationmark").font(.footnote).foregroundStyle(.secondary) }
        }.padding(.horizontal, 20)
    }
    private func recipientSummary(_ message: MailMessage) -> String {
        let recipients = message.to ?? []
        if recipients.count == 1, recipients[0].email.caseInsensitiveCompare(message.accountEmail ?? "") == .orderedSame { return "me" }
        return recipients.isEmpty ? message.accountEmail ?? "you" : recipients.map(\.display).joined(separator: ", ")
    }
    private func bodyFailure(_ reason: String) -> some View {
        VStack(alignment: .leading, spacing: 12) {
            Label("Message couldn’t load", systemImage: "arrow.clockwise.circle").font(.headline)
            Text(reason).font(.subheadline).foregroundStyle(.secondary)
            Button("Try again") { Task { await load() } }.buttonStyle(.bordered).accessibilityIdentifier("retryBody")
        }.padding(20).frame(maxWidth: .infinity, alignment: .leading)
    }
    private func actionBar(_ message: MailMessage) -> some View {
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
            } label: { Image(systemName: "ellipsis").frame(maxWidth: .infinity, minHeight: 44) }.accessibilityLabel("Message actions")
        }.buttonStyle(.plain).labelStyle(.iconOnly).font(.title3).foregroundStyle(Color.accentColor).padding(.horizontal, 8).padding(.vertical, 4)
            .mailGlass(capsule: true).padding(.horizontal, 12).padding(.vertical, 8)
    }
    private func load() async {
        let requestRef = ref
        imageGeneration += 1; let generation = imageGeneration
        message = store.snapshot.messages[requestRef.id]; html = store.snapshot.html[requestRef.id]
        images = false; error = nil; loading = true; webLoading = true
        defer { if generation == imageGeneration { loading = false } }
        do {
            let loaded = try await store.message(requestRef)
            guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
            message = loaded
            if loaded.hasHtml == true {
                let allow = loaded.permitsAutomaticImages
                let body = try await store.imageDocument(requestRef, images: allow, automatic: allow)
                guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
                html = body.html; images = body.imagesAllowed
            }
            guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
            if !loaded.read, await store.action("read", loaded) { message?.read = true }
        } catch { if ref == requestRef, generation == imageGeneration, !Task.isCancelled { self.error = store.friendly(error) } }
    }
    private func loadImages() async {
        let requestRef = ref
        imageGeneration += 1; let generation = imageGeneration
        defer { if generation == imageGeneration { loading = false } }
        do {
            let body = try await store.imageDocument(requestRef, images: true)
            guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
            html = body.html; images = body.imagesAllowed
        } catch { if ref == requestRef, generation == imageGeneration { self.error = store.friendly(error) } }
    }
    private func setImageTrust(_ trusted: Bool) async {
        let requestRef = ref
        imageGeneration += 1; let generation = imageGeneration
        updatingImageTrust = true
        images = false // Revocation takes effect before its network response.
        defer { updatingImageTrust = false; if generation == imageGeneration { loading = false } }
        do {
            var payload = requestRef.payload; payload["trusted"] = trusted
            let policy: ImagePolicy = try await store.api.post("/api/image-trust", body: payload)
            guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
            message?.imagePolicy = policy
            let allow = message?.permitsAutomaticImages == true
            let body = try await store.imageDocument(requestRef, images: allow, automatic: allow)
            guard ref == requestRef, generation == imageGeneration, !Task.isCancelled else { return }
            html = body.html; images = body.imagesAllowed
        } catch { if ref == requestRef, generation == imageGeneration { self.error = store.friendly(error) } }
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
