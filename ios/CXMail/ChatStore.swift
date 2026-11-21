import SwiftUI
import UIKit

struct ChatItem: Identifiable {
    enum Kind { case user, assistant, tool, permission, notice }
    var id = UUID().uuidString
    var kind: Kind
    var text: String
    var detail = ""
    var context = ""
    var streaming = false
    var requestID: String?
    var rememberable = false
    var permissionState = "pending"
    var photos: [UIImage] = []
}

@MainActor @Observable
final class ChatStore {
    let api: APIClient
    var items: [ChatItem] = []
    var id: Int?
    var seed: MessageRef?
    var seq = 0
    var alive = false
    var busy = false
    var starting = false
    var error: String?
    var draft = ""
    /// Photos waiting to go with the next message; like `draft`, they survive docking.
    var photos: [ComposePhoto] = []
    var sending = false
    private var pollTask: Task<Void, Never>?
    private var pollEpoch: UUID?
    init(api: APIClient) { self.api = api }
    func ensure(seed: MessageRef?) async {
        if id == nil { await start(seed: seed) }
        else { resume() }
    }
    func start(seed: MessageRef?) async {
        guard !starting else { return }
        starting = true; error = nil; pause()
        defer { starting = false }
        do {
            // Scope is supplied per turn so choosing another email never
            // restarts the provider or silently clears the conversation.
            let data = try await api.data("/api/chat/start", body: [:])
            let object = try JSONSerialization.jsonObject(with: data) as? [String: Any]
            guard let id = object?["chat"] as? Int else { throw MailError.unexpected }
            self.id = id; self.seed = seed; seq = 0; items = []; alive = true; busy = false
            resume()
        } catch { self.error = error.localizedDescription }
    }
    func pause() { pollTask?.cancel(); pollTask = nil; pollEpoch = nil }
    func resume() {
        guard pollTask == nil, id != nil, alive else { return }
        let epoch = UUID(); pollEpoch = epoch
        pollTask = Task { [weak self] in await self?.poll(epoch: epoch) }
    }
    private func poll(epoch: UUID) async {
        defer { if pollEpoch == epoch { pollTask = nil; pollEpoch = nil } }
        while !Task.isCancelled, let id, alive {
            do {
                let data = try await api.data("/api/chat/events", query: [.init(name: "chat", value: String(id)), .init(name: "after", value: String(seq))])
                guard !Task.isCancelled, self.id == id else { return }
                guard let result = try JSONSerialization.jsonObject(with: data) as? [String: Any] else { throw MailError.unexpected }
                if result["reset"] as? Bool == true {
                    alive = false; busy = false; expirePermissions()
                    error = "This chat was replaced on the mini. Start a new chat."; break
                }
                let events = result["events"] as? [[String: Any]] ?? []
                for wrapper in events {
                    if let event = wrapper["event"] as? [String: Any] { apply(event) }
                    if let error = wrapper["error"] as? String { items.append(.init(kind: .notice, text: error)) }
                }
                seq = result["seq"] as? Int ?? seq
                error = nil
                if result["alive"] as? Bool == false { alive = false; busy = false; expirePermissions(); break }
                if events.isEmpty || api.fixture { try await Task.sleep(for: .milliseconds(500)) }
            } catch {
                if Task.isCancelled { return }
                self.error = "Chat disconnected. Reconnecting to the mini…"
                do { try await Task.sleep(for: .seconds(3)) } catch { return }
            }
        }
    }
    func send(_ text: String, context: MessageRef? = nil, photos: [ComposePhoto] = []) async -> Bool {
        guard !busy, !starting, !sending, !text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty || !photos.isEmpty else { return false }
        if !alive { await start(seed: context) }
        guard alive else { return false }
        busy = true; sending = true; error = nil
        defer { sending = false }
        // Put the outgoing turn before polling can append its streamed reply.
        items.append(.init(kind: .user, text: text, photos: photos.compactMap(\.thumbnail)))
        let request = text.trimmingCharacters(in: .whitespacesAndNewlines).isEmpty ? "(See the attached photo.)" : text
        var body: [String: Any] = ["text": Self.requestText(request, context: context)]
        if !photos.isEmpty { body["images"] = photos.map { $0.data.base64EncodedString() } }
        do {
            let _: OK = try await api.post("/api/chat/send", body: body)
            if draft == text { draft = "" }
            let sent = Set(photos.map(\.id)); self.photos.removeAll { sent.contains($0.id) }
            resume(); return true
        } catch { busy = false; self.error = "Couldn’t confirm delivery. Check the conversation before retrying. " + error.localizedDescription; return false }
    }
    static func requestText(_ text: String, context: MessageRef?) -> String {
        let instruction: String
        if let context {
            let reference: [String: Any] = ["account_id": context.account, "folder": context.folder, "uid": context.uid]
            let data = try! JSONSerialization.data(withJSONObject: reference, options: [.sortedKeys])
            instruction = "Context for this request: the currently selected email, identified by this exact reference: " + String(decoding: data, as: UTF8.self) + ". Inspect it with the mail read/preview tools before answering. Do not substitute an email from an earlier turn."
        } else {
            instruction = "Context for this request: all of my mail. Do not assume an email selected in an earlier turn is the target unless this request refers to it."
        }
        return instruction + "\n\nUser request:\n" + text
    }
    func interrupt() async {
        do { let _: OK = try await api.post("/api/chat/interrupt", body: [:]) }
        catch { self.error = error.localizedDescription }
    }
    func answer(_ item: ChatItem, allow: Bool, remember: Bool) async {
        guard let request = item.requestID, let index = items.firstIndex(where: { $0.id == item.id }), items[index].permissionState == "pending" else { return }
        items[index].permissionState = "answering"
        do {
            let _: OK = try await api.post("/api/chat/answer", body: ["request_id": request, "allow": allow, "remember": remember && item.rememberable])
            items[index].permissionState = allow ? (remember ? "Allowed for this chat" : "Allowed") : "Denied"
        } catch {
            items[index].permissionState = "No longer waiting"; self.error = error.localizedDescription
        }
    }
    func apply(_ event: [String: Any]) {
        let type = event["type"] as? String ?? ""
        let text = event["text"] as? String ?? ""
        switch type {
        case "ready": alive = true
        case "text_delta":
            if items.last?.kind == .assistant && items.last?.streaming == true { items[items.count - 1].text += text }
            else { items.append(.init(kind: .assistant, text: text, streaming: true)) }
        case "assistant_text":
            if items.last?.kind == .assistant && items.last?.streaming == true { items[items.count - 1].text = text; items[items.count - 1].streaming = false }
            else { items.append(.init(kind: .assistant, text: text)) }
        case "tool_use":
            finishStreaming()
            let id = event["id"] as? String ?? UUID().uuidString
            if !items.contains(where: { $0.id == id }) {
                items.append(.init(id: id, kind: .tool, text: event["name"] as? String ?? "Working", detail: Self.pretty(event["input"])))
            }
        case "tool_result":
            if let index = items.firstIndex(where: { $0.id == event["tool_use_id"] as? String }) {
                items[index].detail = text; items[index].text += event["is_error"] as? Bool == true ? " · Failed" : " · Done"
            }
        case "permission_request":
            finishStreaming()
            let request = event["request_id"] as? String ?? ""
            let context = event["context"] as? [String: Any] ?? [:]
            var summary = [String]()
            if let account = context["account"] as? String { summary.append("Account: " + account) }
            if let messages = context["messages"] as? [[String: Any]] {
                summary += messages.map { ($0["subject"] as? String ?? "Message") + " — " + ($0["from"] as? String ?? "") }
                if let total = context["message_count"] as? Int, total > messages.count { summary.append("And \(total - messages.count) more messages") }
            }
            items.append(.init(id: request, kind: .permission, text: event["display_name"] as? String ?? event["tool_name"] as? String ?? "Change mail",
                detail: Self.pretty(["context": event["context"] ?? [:], "input": event["input"] ?? [:]]), context: summary.joined(separator: "\n"), requestID: request, rememberable: event["rememberable"] as? Bool ?? true))
        case "turn_done":
            finishStreaming(); busy = false
            if event["is_error"] as? Bool == true { error = "Claude ended this turn with an error. You can ask it to continue." }
        case "notice": items.append(.init(kind: .notice, text: text))
        case "exited":
            finishStreaming(); alive = false; busy = false; expirePermissions()
            if let code = event["code"] as? Int, code != 0 { error = "Claude stopped unexpectedly. Start a new chat to continue." }
        default: break
        }
    }
    private func finishStreaming() { if !items.isEmpty { items[items.count - 1].streaming = false } }
    private func expirePermissions() {
        for index in items.indices where items[index].kind == .permission && items[index].permissionState == "pending" { items[index].permissionState = "No longer waiting" }
    }
    static func pretty(_ value: Any?) -> String {
        guard let value, JSONSerialization.isValidJSONObject(value), let data = try? JSONSerialization.data(withJSONObject: value, options: [.prettyPrinted, .sortedKeys]) else { return "" }
        return String(data: data, encoding: .utf8) ?? ""
    }
}
