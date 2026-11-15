import SwiftUI

struct ComposerView: View {
    @Bindable var store: MailStore
    @State private var draft: LocalDraft
    @State private var saving = false
    @State private var cancel = false
    @State private var error: String?
    @FocusState private var focus: Field?
    @Environment(\.dismiss) private var dismiss
    enum Field { case to, cc, subject, body }
    init(store: MailStore, initial: LocalDraft) { self.store = store; _draft = State(initialValue: initial) }
    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Picker("From", selection: $draft.account) {
                        ForEach(store.snapshot.accounts) { Text($0.email).tag($0.id) }
                    }.disabled(draft.reply != nil).accessibilityIdentifier("composeFrom")
                    TextField("To", text: $draft.to, axis: .vertical).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled().focused($focus, equals: .to).accessibilityIdentifier("composeTo")
                    TextField("Cc", text: $draft.cc, axis: .vertical).keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled().focused($focus, equals: .cc).accessibilityIdentifier("composeCc")
                    TextField("Subject", text: $draft.subject, axis: .vertical).focused($focus, equals: .subject).accessibilityIdentifier("composeSubject")
                } footer: { Text("Separate email addresses with commas. Your reply is saved on this iPhone as you write.") }
                Section {
                    TextField("Write your message…", text: $draft.body, axis: .vertical)
                        .lineLimit(10...30).focused($focus, equals: .body).accessibilityIdentifier("composeBody")
                }
                if let error {
                    Section { Label(error, systemImage: "exclamationmark.circle").font(.subheadline).foregroundStyle(.red).accessibilityIdentifier("composeError") }
                }
                Section {
                    Button { Task { await save(review: false) } } label: { Label("Save to Drafts on mini", systemImage: "doc.badge.arrow.up") }.disabled(!draft.valid || saving).accessibilityIdentifier("saveDraft")
                } footer: { Text("Saving does not send. Review the complete draft before sending from the next screen.") }
            }
            .navigationTitle(draft.reply == nil ? "New message" : "Reply").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { cancel = true }.disabled(saving).accessibilityIdentifier("cancelCompose") }
                ToolbarItem(placement: .confirmationAction) {
                    Button { Task { await save(review: true) } } label: {
                        if saving { ProgressView() } else { Text("Review").bold() }
                    }.disabled(!draft.valid || saving).accessibilityIdentifier("reviewCompose")
                }
                ToolbarItemGroup(placement: .keyboard) {
                    Spacer(); Button("Done") { focus = nil }
                }
            }
            .interactiveDismissDisabled(draft.hasContent || saving)
            .confirmationDialog("Keep your reply?", isPresented: $cancel, titleVisibility: .visible) {
                Button("Keep on this iPhone") { store.remember(draft); store.composer = nil; dismiss() }
                Button("Discard reply", role: .destructive) { store.clearDraft(); dismiss() }
                Button("Continue writing", role: .cancel) { }
            }
            .onChange(of: draft) { _, value in store.composer = value; store.remember(value) }
        }
    }
    private func save(review: Bool) async {
        guard !saving else { return }
        focus = nil; saving = true; error = nil
        do {
            let result = try await store.saveDraft(draft)
            dismiss()
            // Wait for the composer to dismiss before presenting the review sheet.
            if review, let ref = result.draft {
                Task { @MainActor in try? await Task.sleep(for: .milliseconds(400)); store.review = ref }
            } else { store.notice = "Saved to Drafts on the mini." }
            Task { await store.load() }
        } catch {
            self.error = store.friendly(error) + " Your reply is still saved on this iPhone. If the connection dropped while saving, check Drafts before trying again."
        }
        saving = false
    }
}

struct ReviewView: View {
    let store: MailStore
    let ref: MessageRef
    var embedded = false
    @State private var preview: DraftPreview?
    @State private var html: String?
    @State private var height: CGFloat = 240
    @State private var error: String?
    @State private var loading = true
    @State private var sending = false
    @State private var attempted = false
    @State private var confirm = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        Group {
            if embedded { content }
            else { NavigationStack { content.toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() }.disabled(sending) } } } }
        }
        .task(id: ref) { await load() }
    }
    private var content: some View {
        ScrollView {
            VStack(alignment: .leading, spacing: 20) {
                if let preview {
                    VStack(alignment: .leading, spacing: 16) {
                        Text(preview.subject.isEmpty ? "(no subject)" : preview.subject).font(.title2.bold()).textSelection(.enabled)
                        AddressLine(title: "From", addresses: [preview.from])
                        AddressLine(title: "To", addresses: preview.to)
                        AddressLine(title: "Cc", addresses: preview.cc)
                        AddressLine(title: "Bcc · hidden recipients", addresses: preview.bcc).accessibilityIdentifier("reviewBcc")
                        if let blocked = preview.blockedReason { Label(blocked, systemImage: "exclamationmark.shield").foregroundStyle(.red) }
                        if !preview.attachments.isEmpty {
                            Text("Attachments: " + preview.attachments.joined(separator: ", ")).font(.subheadline)
                        }
                        Label("Remote images blocked", systemImage: "eye.slash").font(.caption).foregroundStyle(.secondary)
                    }.padding(.horizontal, 20)
                    if let html { EmailHTMLView(html: html, height: $height).frame(height: height) }
                }
                if loading { ProgressView("Fetching draft from the mini…").frame(maxWidth: .infinity).padding(20) }
                if let error {
                    VStack(alignment: .leading, spacing: 12) {
                        Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red).accessibilityIdentifier("sendError")
                        Button("Refresh review") { Task { await load() } }.disabled(sending)
                    }.padding(20)
                }
            }.padding(.vertical, 20)
        }
        .navigationTitle("Review before sending").navigationBarTitleDisplayMode(.inline)
        .safeAreaInset(edge: .bottom) {
            VStack(spacing: 8) {
                Button { confirm = true } label: {
                    HStack { if sending { ProgressView() }; Label(sending ? "Sending…" : "Send", systemImage: "paperplane.fill").frame(maxWidth: .infinity) }.frame(minHeight: 44)
                }
                .buttonStyle(.borderedProminent)
                .disabled(loading || sending || attempted || preview?.sendable != true || html == nil)
                .accessibilityIdentifier("sendReviewedDraft")
                Text("Only the exact draft you reviewed can be sent.").font(.caption).foregroundStyle(.secondary)
            }.padding(16).background(.bar)
        }
        .interactiveDismissDisabled(sending)
        .confirmationDialog("Send this reviewed draft?", isPresented: $confirm, titleVisibility: .visible) {
            Button("Send now") { Task { await send() } }.accessibilityIdentifier("confirmSend")
            Button("Cancel", role: .cancel) { }
        } message: {
            if let preview { Text("From \(preview.from.email) to \(preview.to.count + preview.cc.count + preview.bcc.count) recipients, including \(preview.bcc.count) Bcc. This sends immediately.") }
        }
    }
    private func load() async {
        loading = true; error = nil; html = nil; preview = nil; attempted = false
        defer { loading = false }
        do {
            let p: DraftPreview = try await store.api.get("/api/draft-preview", query: ref.query)
            let body = try await store.api.html("/api/draft-body", query: p.ref.query + [.init(name: "digest", value: p.digest)])
            guard !Task.isCancelled else { return }
            preview = p; html = body
        } catch { if !Task.isCancelled { self.error = store.friendly(error) } }
    }
    private func send() async {
        guard let preview, preview.sendable, html != nil, !sending, !attempted else { return }
        sending = true; attempted = true; error = nil
        defer { sending = false }
        do {
            var body = preview.ref.payload; body["digest"] = preview.digest
            let result: SendResult = try await store.api.post("/api/send", body: body)
            store.snapshot.lists = [:]; store.persist(); store.review = nil; store.selected = nil
            dismiss(); store.notice = result.draftRemoved ? "Sent." : result.note ?? "Sent. Check Drafts on the Mac for the remaining copy."
            await store.load()
        } catch {
            // Never silently retry a send: a dropped reply may follow successful SMTP.
            self.error = store.friendly(error) + " Check Sent before trying again. Refresh the review to continue."
        }
    }
}
