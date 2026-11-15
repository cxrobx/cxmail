import SwiftUI

struct ScheduleSendView: View {
    let store: MailStore
    let preview: DraftPreview
    let onScheduled: () -> Void
    @State private var date = Calendar.current.nextDate(after: .now, matching: .init(hour: 9), matchingPolicy: .nextTime) ?? .now.addingTimeInterval(3600)
    @State private var confirm = false
    @State private var saving = false
    @State private var attempted = false
    @State private var error: String?
    @State private var showQueue = false
    @Environment(\.dismiss) private var dismiss

    var body: some View {
        NavigationStack {
            Form {
                Section {
                    Text(preview.subject.isEmpty ? "(no subject)" : preview.subject).font(.headline)
                    AddressLine(title: "From", addresses: [preview.from])
                    AddressLine(title: "To", addresses: preview.to)
                    AddressLine(title: "Cc", addresses: preview.cc)
                    AddressLine(title: "Bcc · hidden recipients", addresses: preview.bcc)
                }
                Section {
                    Button("In one hour") { date = .now.addingTimeInterval(3600) }
                    Button("Tomorrow at 9 AM") {
                        let tomorrow = Calendar.current.date(byAdding: .day, value: 1, to: .now) ?? .now
                        date = Calendar.current.date(bySettingHour: 9, minute: 0, second: 0, of: tomorrow) ?? tomorrow
                    }
                    DatePicker("Send at", selection: $date, in: Date.now.addingTimeInterval(60)...Date.now.addingTimeInterval(365 * 86400))
                        .accessibilityIdentifier("scheduledDate")
                    Text(TimeZone.current.identifier).font(.caption).foregroundStyle(.secondary)
                } footer: {
                    Text("The mini must be awake and online. If asleep, it will try after waking. Changes to this reviewed draft stop delivery.")
                }.disabled(saving || attempted)
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red).accessibilityIdentifier("scheduleError")
                        Button("Check Scheduled") { showQueue = true }
                    }
                }
            }
            .navigationTitle("Schedule send").navigationBarTitleDisplayMode(.inline)
            .toolbar {
                ToolbarItem(placement: .cancellationAction) { Button("Cancel") { dismiss() }.disabled(saving) }
                ToolbarItem(placement: .confirmationAction) {
                    Button { confirm = true } label: {
                        if saving { ProgressView() } else { Text("Schedule").fontWeight(.semibold) }
                    }.disabled(saving || attempted || date.timeIntervalSinceNow < 30).accessibilityIdentifier("scheduleDraft")
                }
            }
            .confirmationDialog("Schedule this reviewed draft?", isPresented: $confirm, titleVisibility: .visible) {
                Button("Schedule send") { Task { await save() } }.accessibilityIdentifier("confirmSchedule")
                Button("Cancel", role: .cancel) { }
            } message: {
                Text("Send \(date.formatted(date: .complete, time: .shortened)) to \(preview.to.count + preview.cc.count + preview.bcc.count) recipients, including \(preview.bcc.count) Bcc. Only this exact reviewed draft is approved.")
            }
            .sheet(isPresented: $showQueue) { ScheduledSendsView(store: store) }
        }
        .interactiveDismissDisabled(saving)
    }
    private func save() async {
        guard !saving && !attempted else { return }
        saving = true; attempted = true; error = nil
        defer { saving = false }
        do {
            var body = preview.ref.payload
            body["digest"] = preview.digest
            body["send_at"] = ISO8601DateFormatter().string(from: date)
            let _: ScheduledSend = try await store.api.post("/api/schedule", body: body)
            onScheduled()
        } catch {
            self.error = store.friendly(error) + " Check Scheduled before trying again."
        }
    }
}

struct ScheduledSendsView: View {
    let store: MailStore
    @State private var rows: [ScheduledSend] = []
    @State private var loading = true
    @State private var error: String?
    @State private var cancelling: ScheduledSend?
    @State private var busy = false
    @Environment(\.dismiss) private var dismiss
    var body: some View {
        NavigationStack {
            List {
                if let error {
                    Section {
                        Label(error, systemImage: "exclamationmark.circle").foregroundStyle(.red)
                        Button("Refresh status") { Task { await load() } }
                    }
                }
                if loading && rows.isEmpty { ProgressView("Loading scheduled sends…") }
                else if rows.isEmpty {
                    ContentUnavailableView("No scheduled sends", systemImage: "clock", description: Text("Review a draft and choose Schedule send from its send options."))
                }
                ForEach(rows) { row in
                    VStack(alignment: .leading, spacing: 8) {
                        Text(row.subject).font(.headline)
                        Text(row.recipients).font(.subheadline).foregroundStyle(.secondary)
                        Label(row.title, systemImage: row.status == "uncertain" ? "exclamationmark.circle" : "clock")
                            .font(.subheadline).foregroundStyle(row.status == "uncertain" ? Color.red : Color.secondary)
                        if let date = row.date { Text(date, format: .dateTime.month(.abbreviated).day().hour().minute()).font(.subheadline).monospacedDigit() }
                        if let note = row.note { Text(note).font(.footnote).foregroundStyle(.secondary) }
                        if row.status == "pending" {
                            Button("Cancel scheduled send", role: .destructive) { cancelling = row }
                                .disabled(busy).accessibilityIdentifier("cancelSchedule-\(row.id)")
                        }
                    }.padding(.vertical, 4)
                }
            }
            .navigationTitle("Scheduled sends").navigationBarTitleDisplayMode(.inline)
            .toolbar { ToolbarItem(placement: .confirmationAction) { Button("Done") { dismiss() }.disabled(busy).accessibilityIdentifier("closeScheduled") } }
            .refreshable { await load() }
            .task {
                while !Task.isCancelled {
                    await load()
                    do { try await Task.sleep(for: .seconds(10)) } catch { return }
                }
            }
            .confirmationDialog("Cancel scheduled send?", isPresented: Binding(get: { cancelling != nil }, set: { if !$0 { cancelling = nil } }), titleVisibility: .visible) {
                if let row = cancelling {
                    Button("Cancel scheduled send", role: .destructive) { Task { await cancel(row) } }.accessibilityIdentifier("confirmCancelSchedule")
                }
                Button("Keep scheduled", role: .cancel) { cancelling = nil }
            } message: { Text("The email stays in Drafts. Nothing will be sent by this schedule.") }
        }
        .interactiveDismissDisabled(busy)
    }
    private func load() async {
        defer { loading = false }
        do { rows = try await store.api.get("/api/scheduled"); error = nil }
        catch { if !Task.isCancelled { self.error = store.friendly(error) } }
    }
    private func cancel(_ row: ScheduledSend) async {
        busy = true; cancelling = nil
        defer { busy = false }
        do { let _: OK = try await store.api.post("/api/scheduled/cancel", body: ["id": row.id]); await load() }
        catch { self.error = store.friendly(error) + " Refresh status before trying again." }
    }
}
