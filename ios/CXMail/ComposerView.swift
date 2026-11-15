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
    @State private var schedule = false
    @State private var scheduled = false
    @State private var currentSchedule: ScheduledSend?
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        Group {
            if embedded { content }
            else { NavigationStack { content.toolbar { ToolbarItem(placement: .cancellationAction) { Button("Done") { dismiss() }.disabled(sending) } } } }
        }
        .task(id: ref) { await load() }
        .sheet(isPresented: $schedule, onDismiss: { Task { await load() } }) {
            if let preview {
                ScheduleSendView(store: store, preview: preview) {
                    schedule = false; attempted = true
                }
            }
        }
        .sheet(isPresented: $scheduled, onDismiss: { Task { await load() } }) { ScheduledSendsView(store: store) }
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
                        if let currentSchedule {
                            Button { scheduled = true } label: {
                                Label {
                                    if let date = currentSchedule.date { Text("Scheduled for \(date.formatted(date: .abbreviated, time: .shortened))") }
                                    else { Text(currentSchedule.title) }
                                } icon: { Image(systemName: "clock") }
                            }.accessibilityIdentifier("reviewSchedule")
                        }
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
            HStack {
                Spacer(minLength: 0)
                HStack(spacing: 0) {
                    Button { confirm = true } label: {
                        ZStack {
                            if sending { ProgressView() }
                            else { Image(systemName: "paperplane.fill").font(.title3) }
                        }.frame(width: 48, height: 44).contentShape(Rectangle())
                    }
                    .disabled(!canSend)
                    .accessibilityLabel(sending ? "Sending…" : "Send now")
                    .accessibilityHint("Review the final confirmation before sending")
                    .accessibilityIdentifier("sendReviewedDraft")
                    Divider().frame(height: 20).padding(.horizontal, 4)
                    Menu {
                        Button("Send now", systemImage: "paperplane") { confirm = true }.disabled(!canSend)
                        Button("Schedule send…", systemImage: "clock") { schedule = true }.disabled(!canSend)
                        Button("Scheduled sends", systemImage: "clock.arrow.circlepath") { scheduled = true }
                    } label: {
                        Image(systemName: "chevron.down").font(.subheadline.weight(.semibold)).frame(width: 44, height: 44).contentShape(Rectangle())
                    }
                    .disabled(sending).accessibilityLabel("Send options").accessibilityIdentifier("sendOptions")
                }
                .buttonStyle(.plain).foregroundStyle(Color.accentColor)
                .padding(4).mailGlass(capsule: true)
            }.padding(.horizontal, 16).padding(.vertical, 8)
        }
        .interactiveDismissDisabled(sending)
        .confirmationDialog("Send this reviewed draft?", isPresented: $confirm, titleVisibility: .visible) {
            Button("Send now") { Task { await send() } }.accessibilityIdentifier("confirmSend")
            Button("Cancel", role: .cancel) { }
        } message: {
            if let preview { Text("From \(preview.from.email) to \(preview.to.count + preview.cc.count + preview.bcc.count) recipients, including \(preview.bcc.count) Bcc. This sends immediately.") }
        }
    }
    private var canSend: Bool { !loading && !sending && !attempted && currentSchedule == nil && preview?.sendable == true && html != nil }
    private func load() async {
        loading = true; error = nil; html = nil; preview = nil; attempted = false; currentSchedule = nil
        defer { loading = false }
        do {
            let p: DraftPreview = try await store.api.get("/api/draft-preview", query: ref.query)
            let body = try await store.api.html("/api/draft-body", query: p.ref.query + [.init(name: "digest", value: p.digest)])
            let queue: [ScheduledSend] = try await store.api.get("/api/scheduled")
            guard !Task.isCancelled else { return }
            preview = p; html = body; currentSchedule = queue.first { $0.ref == p.ref && $0.active }
        } catch MailError.draftUnavailable {
            guard !Task.isCancelled else { return }
            await store.refreshAfterMissingDraft(ref)
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
        } catch MailError.draftUnavailable {
            await store.refreshAfterMissingDraft(ref)
        } catch {
            // Never silently retry a send: a dropped reply may follow successful SMTP.
            self.error = store.friendly(error) + " Check Sent before trying again. Refresh the review to continue."
        }
    }
}
