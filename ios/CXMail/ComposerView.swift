import SwiftUI

struct ComposerView: View {
    @Bindable var store: MailStore
    @State private var draft: LocalDraft
    @State private var saving = false
    @State private var cancel = false
    @State private var error: String?
    @State private var showCopies: Bool
    @State private var context: ComposeContext?
    @State private var contextAccount: String?
    @State private var contextFailed = false
    @State private var contextHeight: CGFloat = 1
    @State private var attachment: DraftAttachment?
    @State private var photos: [ComposePhoto]
    /// The body is a UIKit text view (so it accepts pasted images); it reports focus here.
    @State private var bodyFocused = false
    @ScaledMetric(relativeTo: .body) private var labelWidth: CGFloat = 64
    @FocusState private var focus: Field?
    @Environment(\.dismiss) private var dismiss
    @Environment(\.dynamicTypeSize) private var typeSize
    enum Field { case to, cc, bcc, subject, body }

    init(store: MailStore, initial: LocalDraft) {
        self.store = store
        _draft = State(initialValue: initial)
        _showCopies = State(initialValue: !initial.cc.isEmpty || !(initial.bcc ?? "").isEmpty)
        _photos = State(initialValue: store.loadPhotos(initial.photos ?? []) + store.loadFiles(initial.files ?? []))
    }

    var body: some View {
        NavigationStack {
            ScrollView {
                VStack(alignment: .leading, spacing: 0) {
                    headers
                    writingArea
                    attachments
                    if let error {
                        Label(error, systemImage: "exclamationmark.circle")
                            .font(.subheadline).foregroundStyle(.red)
                            .padding(16).accessibilityIdentifier("composeError")
                    }
                }
            }
            .scrollDismissesKeyboard(.interactively)
            .background(Color.mailPaper)
            .safeAreaInset(edge: .bottom, spacing: 0) {
                if !editing {
                    VStack(spacing: 0) {
                        Divider()
                        HStack {
                            Button { Task { await save(review: false) } } label: {
                                Label("Save draft", systemImage: "square.and.arrow.down")
                                    .font(.subheadline.weight(.medium))
                                    .frame(minHeight: 44)
                            }
                            .disabled(!draft.valid || saving)
                            .accessibilityIdentifier("saveDraft")
                            Spacer(minLength: 0)
                        }
                        .padding(.horizontal, 16)
                    }.background(Color.mailPaper)
                }
            }
            .navigationTitle(draft.revision != nil ? "Edit draft" : draft.reply == nil ? "New message" : "Reply")
            .navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { cancel = true }.disabled(saving).accessibilityIdentifier("cancelCompose") }
                ToolbarItem(placement: .confirmationAction) {
                    Button { Task { await save(review: true) } } label: {
                        if saving { ProgressView() } else { Text("Review").bold() }
                    }.disabled(!draft.valid || saving).accessibilityIdentifier("reviewCompose")
                }
                // A keyboard accessory toolbar can drop the navigation actions on Duo.
                ToolbarItem(placement: .topBarTrailing) {
                    if editing {
                        Button("Hide keyboard", systemImage: "keyboard.chevron.compact.down") { focus = nil; bodyFocused = false }
                            .accessibilityIdentifier("hideComposeKeyboard")
                    }
                }
            }
            .interactiveDismissDisabled(draft.hasContent || saving)
            .confirmationDialog("Keep your changes?", isPresented: $cancel, titleVisibility: .visible) {
                Button("Keep on this iPhone") { store.remember(draft); store.composer = nil; dismiss() }
                Button("Discard changes", role: .destructive) { store.clearDraft(draft); dismiss() }
                Button("Continue writing", role: .cancel) { }
            }
            .sheet(item: $attachment) { item in
                DraftAttachmentView(api: store.api, item: item) { failure in
                    attachment = nil
                    error = store.friendly(failure)
                }
            }
            .onChange(of: draft) { _, value in
                store.composer = value
                store.remember(value)
            }
            .onChange(of: photos) { old, new in
                let kept = Set(new.map(\.id))
                for photo in new where !old.contains(photo) {
                    do { try store.storePhoto(photo) } catch { self.error = "Couldn’t keep that photo on this phone. Save the draft before leaving." }
                }
                for photo in old where !kept.contains(photo.id) { store.removePhoto(photo.id) }
                let images = new.filter(\.isImage), files = new.filter { !$0.isImage }
                draft.photos = images.isEmpty ? nil : images.map(\.id)
                draft.files = files.isEmpty ? nil : files.map { DraftFile(id: $0.id, name: $0.name) }
            }
            .task(id: draft.account) { await loadContext() }
        }
    }

    private var headers: some View {
        VStack(alignment: .leading, spacing: 0) {
            headerRow("From") {
                if draft.reply == nil, draft.revision == nil, store.snapshot.accounts.count > 1 {
                    Menu {
                        Picker("From address", selection: $draft.account) {
                            ForEach(store.snapshot.accounts) { Text($0.email).tag($0.id) }
                        }
                    } label: {
                        HStack(alignment: .top, spacing: 6) {
                            Text(fromAddress).fixedSize(horizontal: false, vertical: true)
                                .multilineTextAlignment(.leading)
                            Image(systemName: "chevron.down").font(.caption.weight(.semibold))
                                .accessibilityHidden(true)
                        }
                        .frame(maxWidth: .infinity, alignment: .leading)
                        .fixedSize(horizontal: false, vertical: true)
                    }
                    .accessibilityLabel("From address")
                    .accessibilityValue(fromAddress)
                    .accessibilityIdentifier("composeFrom")
                } else {
                    Text(fromAddress).fixedSize(horizontal: false, vertical: true)
                        .textSelection(.enabled)
                }
            }
            headerRow("To") {
                recipient("To", text: $draft.to, field: .to, id: "composeTo")
                if !showCopies {
                    Button("Cc/Bcc") { showCopies = true }
                        .font(.subheadline.weight(.medium))
                        .frame(minHeight: 44)
                        .accessibilityLabel("Show Cc and Bcc")
                        .accessibilityIdentifier("showComposeCopies")
                }
            }
            if showCopies {
                headerRow("Cc") { recipient("Cc", text: $draft.cc, field: .cc, id: "composeCc") }
                headerRow("Bcc") {
                    recipient("Bcc", text: Binding(get: { draft.bcc ?? "" }, set: { draft.bcc = $0 }), field: .bcc, id: "composeBcc")
                }
            }
            headerRow("Subject") {
                TextField("Subject", text: $draft.subject, prompt: Text(""), axis: .vertical)
                    .focused($focus, equals: .subject).accessibilityIdentifier("composeSubject")
            }
        }
    }

    private func headerRow<Content: View>(_ title: String, @ViewBuilder content: () -> Content) -> some View {
        let stacked = typeSize.isAccessibilitySize
        let layout = stacked
            ? AnyLayout(VStackLayout(alignment: .leading, spacing: 4))
            : AnyLayout(HStackLayout(alignment: .firstTextBaseline, spacing: 12))
        return VStack(spacing: 0) {
            layout {
                Text(title).foregroundStyle(.secondary)
                    .frame(width: stacked ? nil : labelWidth, alignment: .leading)
                    .accessibilityHidden(true)
                layout { content() }
            }
            .frame(maxWidth: .infinity, minHeight: 44, alignment: .leading)
            .padding(.horizontal, 16)
            .padding(.vertical, stacked ? 6 : 0)
            Divider()
        }
    }

    private func recipient(_ title: String, text: Binding<String>, field: Field, id: String) -> some View {
        TextField(title, text: text, prompt: Text(""), axis: .vertical)
            .keyboardType(.emailAddress).textInputAutocapitalization(.never).autocorrectionDisabled()
            .focused($focus, equals: field).accessibilityIdentifier(id)
    }

    private var writingArea: some View {
        VStack(alignment: .leading, spacing: 0) {
            PastingTextView(text: $draft.body, isFocused: $bodyFocused, placeholder: "Write your message…",
                            minLines: 3, identifier: "composeBody") { images in
                if canAddPhotos {
                    if let problem = PhotoEncoder.add(images: images, to: &photos, limit: 10, maxPixel: PhotoEncoder.mailPixels) { error = problem }
                } else {
                    error = "Photos can be added to new messages and replies. Send this draft from the Mac to add one."
                }
            }
            if canAddPhotos {
                // Between the text and the signature, so it stays in view while typing.
                photoControls.padding(.leading, -12)
                if !photos.isEmpty { PhotoStrip(photos: $photos).padding(.bottom, 8) }
            }
            if let html = displayedContext?.html, !html.isEmpty {
                EmailHTMLView(html: html, height: $contextHeight, inComposer: true)
                    .frame(height: contextHeight)
                    .accessibilityIdentifier("composeSignature")
            } else if contextFailed {
                Button("Reload signature preview", systemImage: "arrow.clockwise") { Task { await loadContext() } }
                    .font(.subheadline)
            }
        }
        .padding(.horizontal, 16)
        .padding(.top, 16)
        .padding(.bottom, 20)
        .frame(maxWidth: .infinity, alignment: .leading)
    }

    @ViewBuilder private var attachments: some View {
        if let revision = draft.revision, !revision.attachments.isEmpty {
            Divider()
            VStack(alignment: .leading, spacing: 8) {
                ForEach(revision.attachments.indices, id: \.self) { index in
                    let item = DraftAttachment(index: index, name: revision.attachments[index], ref: revision.ref, digest: revision.digest)
                    Button { focus = nil; bodyFocused = false; attachment = item } label: { DraftAttachmentTile(item: item) }
                        .buttonStyle(.plain)
                        .accessibilityLabel(item.name)
                        .accessibilityHint("Preview this attachment")
                }
            }.padding(16)
        }
    }

    /// New messages and replies only: an already-saved draft's MIME is rebuilt
    /// in place on the mini, which keeps its files but cannot add new ones yet.
    private var canAddPhotos: Bool { draft.revision == nil }

    private var photoControls: some View {
        PhotoAddControls(photos: $photos, limit: 10, maxPixel: PhotoEncoder.mailPixels, anyFile: true) { error = $0 }
    }

    private var editing: Bool { focus != nil || bodyFocused }

    private var displayedContext: ComposeContext? {
        contextAccount == draft.account ? context : nil
    }

    private var fromAddress: String {
        draft.revision?.from ?? displayedContext?.from
            ?? store.snapshot.accounts.first(where: { $0.id == draft.account })?.email ?? ""
    }

    private func loadContext() async {
        let account = draft.account
        guard !account.isEmpty else { return }
        var query = (draft.revision?.ref ?? draft.reply)?.query ?? [.init(name: "account", value: account)]
        if let revision = draft.revision { query.append(.init(name: "digest", value: revision.digest)) }
        contextFailed = false
        do {
            let value: ComposeContext = try await store.api.get("/api/compose-context", query: query)
            guard !Task.isCancelled, account == draft.account else { return }
            contextHeight = 1
            context = value
            contextAccount = account
        } catch {
            guard !Task.isCancelled, account == draft.account else { return }
            contextFailed = true
        }
    }

    private func save(review: Bool) async {
        guard !saving else { return }
        focus = nil; bodyFocused = false; saving = true; error = nil
        do {
            let result = try await store.saveDraft(draft)
            dismiss()
            // Wait for the composer to dismiss before presenting the review sheet.
            if let ref = result.draft, let old = draft.revision?.ref, store.selected == old {
                store.selected = ref
            }
            if review, let ref = result.draft {
                store.notice = result.result
                if store.selected != ref {
                    Task { @MainActor in try? await Task.sleep(for: .milliseconds(400)); store.review = ref }
                }
            } else { store.notice = draft.revision == nil ? "Saved to Drafts on the mini." : result.result ?? "Draft updated." }
            Task { await store.load() }
        } catch {
            self.error = store.friendly(error) + " Your changes are still saved on this iPhone. If the connection dropped while saving, check Drafts before trying again."
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
    @State private var attachment: DraftAttachment?
    @State private var reviewExpired = false
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
        .sheet(item: $attachment) { item in
            DraftAttachmentView(api: store.api, item: item) { failure in
                attachment = nil
                switch failure {
                case .draftUnavailable:
                    Task { await store.refreshAfterMissingDraft(item.ref) }
                default:
                    reviewExpired = true
                    error = store.friendly(failure)
                }
            }
        }
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
                        if reviewExpired, let error {
                            Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red)
                                .accessibilityIdentifier("sendError")
                            Button("Refresh review") { Task { await load() } }
                        }
                        if let editable = preview.editText, currentSchedule == nil {
                            Button("Edit draft", systemImage: "square.and.pencil") {
                                if let recovered = store.recoveredEdits[preview.ref.id],
                                   recovered.revision?.digest == preview.digest {
                                    openEditor(recovered)
                                    return
                                }
                                openEditor(LocalDraft(
                                    account: preview.accountId,
                                    to: preview.to.map(\.email).joined(separator: ", "),
                                    cc: preview.cc.map(\.email).joined(separator: ", "),
                                    subject: preview.subject, body: editable,
                                    revision: .init(ref: preview.ref, digest: preview.digest,
                                                    from: preview.from.email, attachments: preview.attachments),
                                    bcc: preview.bcc.map(\.email).joined(separator: ", ")
                                ))
                            }
                            .disabled(reviewExpired || loading || sending || attempted)
                            .accessibilityIdentifier("editReviewedDraft")
                        }
                        if !preview.attachments.isEmpty {
                            VStack(alignment: .leading, spacing: 8) {
                                Text("Attachments").font(.footnote.weight(.semibold)).foregroundStyle(.secondary)
                                ForEach(preview.attachments.indices, id: \.self) { index in
                                    let item = DraftAttachment(
                                        index: index, name: preview.attachments[index], ref: preview.ref, digest: preview.digest
                                    )
                                    Button { attachment = item } label: { DraftAttachmentTile(item: item) }
                                        .buttonStyle(.plain)
                                        .disabled(reviewExpired || loading || sending)
                                        .accessibilityLabel(item.name)
                                        .accessibilityHint("Preview this attachment")
                                        .accessibilityIdentifier("reviewAttachment-\(index)")
                                }
                            }
                            .frame(maxWidth: 340, alignment: .leading)
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
                if let error, !reviewExpired {
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
    private var canSend: Bool { !loading && !sending && !attempted && !reviewExpired && currentSchedule == nil && preview?.sendable == true && html != nil }
    private func openEditor(_ draft: LocalDraft) {
        guard store.composer == nil else { return }
        if store.review == nil {
            store.composer = draft
        } else {
            store.review = nil
            Task { @MainActor in
                try? await Task.sleep(for: .milliseconds(400))
                if store.composer == nil { store.composer = draft }
            }
        }
    }

    private func load() async {
        loading = true; error = nil; html = nil; preview = nil; attempted = false; reviewExpired = false; currentSchedule = nil
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
        guard canSend, let preview else { return }
        sending = true; attempted = true; error = nil
        defer { sending = false }
        do {
            var body = preview.ref.payload; body["digest"] = preview.digest
            let result: SendResult = try await store.api.post("/api/send", body: body)
            store.snapshot.lists = [:]; store.persist(); store.review = nil; store.selected = nil
            dismiss(); store.notice = result.draftRemoved ? "Sent." : result.note ?? "Sent. Check Drafts on the Mac for the remaining copy."
            await store.load()
        } catch MailError.draftUnavailable {
            await store.refreshAfterMissingDraft(preview.ref)
        } catch {
            // Never silently retry a send: a dropped reply may follow successful SMTP.
            self.error = store.friendly(error) + " Check Sent before trying again. Refresh the review to continue."
        }
    }
}
