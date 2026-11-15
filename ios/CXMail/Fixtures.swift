#if DEBUG
import Foundation

/// Fiction-only, in-process transport. UI tests can never touch a real mailbox.
@MainActor enum Fixtures {
    static var archived = Set<Int>()
    static var starred = Set<Int>()
    static var read = Set<Int>()
    static var saved = false
    static var sent = false
    static var revision = 0
    static var bodyRequests = 0
    static var chatStarts = 0
    static var lastChatText = ""
    static var scheduled: [[String: Any]] = []
    static var missingDrafts = Set<Int>()
    static var draftPreviewRequests = 0
    static func message(_ uid: Int = 10, detail: Bool = false) -> [String: Any] {
        var value: [String: Any] = ["account": "work", "folder": "INBOX", "uid": uid,
            "subject": uid == 10 ? "A good place to pick this up" : "The next steps for our project",
            "from_name": uid == 10 ? "Jordan Lee" : "Alex Morgan", "from_email": "jordan@example.org",
            "date": "2026-11-14T17:30:00+00:00", "snippet": "Thanks for the thoughtful notes. Here’s what I’m thinking for next week.",
            "read": read.contains(uid), "starred": starred.contains(uid), "attachments": true, "category": "primary", "recipients": ["Chris"]]
        let examples: [Int: (String, String, String)] = [
            12: ("Bluebird Billing", "Your November statement is ready", ""),
            13: ("Morgan Chen", "Re: Thursday’s session", "Thursday at 2 works for me. I’ll send the notes over beforehand."),
            14: ("Northline Travel", "Your trip receipt", "Thanks for riding with us. Your receipt is attached."),
            15: ("Casey Brooks", "A couple of ideas for the launch", "I pulled together a short list after our conversation yesterday."),
            16: ("Field Notes", "Something worth reading this weekend", "A few things we’ve been thinking about lately."),
            17: ("Jamie Rivera", "Files for next week", "Here’s everything in one place. Let me know if anything is missing.")
        ]
        if let example = examples[uid] { value["from_name"] = example.0; value["subject"] = example.1; value["snippet"] = example.2; value["attachments"] = uid == 12 || uid == 14 }
        if detail {
            value.merge(["to": [["email": "me@example.com", "name": "Chris"]], "cc": [["email": "alex@example.org", "name": "Alex"]],
                "cached": true, "has_html": true, "attachment_names": ["Project outline.pdf"], "thread": uid == 10 ? [message(11)] : [],
                "account_email": "me@example.com", "images_trusted": false], uniquingKeysWith: { _, new in new })
        }
        return value
    }
    static var imageSenderTrusted = false
    static func imagePolicy() -> [String: Any] {
        let failed = ProcessInfo.processInfo.arguments.contains("--image-auth-failed")
        return ["authentication": ["status": failed ? "failed" : "passed", "domain": "example.org", "provider": "Google", "explanation": "The sender domain was checked. This does not verify links or attachments."],
                "sender_email": "jordan@example.org", "sender_trusted": imageSenderTrusted,
                "auto_load": imageSenderTrusted && !failed, "suspicious": failed,
                "has_remote_images": true, "blocked_reason": "Images require your permission."]
    }
    static let html = "<html><head><meta name='viewport' content='width=device-width,initial-scale=1'></head><body style='font:17px -apple-system;line-height:1.6;padding:20px;color:#202124;background:white'><p>Hi Chris,</p><p>Thanks for the thoughtful notes. I’ve had a chance to work through the outline, and I think we have a good place to pick this up.</p><p>Could we find a little time next week to talk through the next steps? Tuesday or Thursday would work well on my end.</p><p>I especially liked the idea of keeping the first version focused. Let’s start with what we’ll actually use, then take it from there.</p><p>Talk soon,<br>Jordan</p></body></html>"
    static func response(_ path: String, query: [URLQueryItem], body: [String: Any]?) throws -> Data {
        let q = Dictionary(query.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, n in n })
        var result: Any = ["ok": true]
        switch path {
        case "/api/accounts": result = [["id": "work", "email": "me@example.com", "name": "Work", "hidden": false], ["id": "personal", "email": "personal@example.com", "name": "Personal", "hidden": false]]
        case "/api/messages":
            if ProcessInfo.processInfo.arguments.contains("--offline") { throw URLError(.notConnectedToInternet) }
            if ProcessInfo.processInfo.arguments.contains("--empty") || (q["q"] != nil && q["q"] != "project") { result = [[String: Any]]() }
            else { result = (10...17).map { message($0) }.filter { !archived.contains($0["uid"] as! Int) && (q["view"] != "unread" || !($0["read"] as! Bool)) && (q["view"] != "starred" || ($0["starred"] as! Bool)) } }
        case "/api/message":
            var body = message(Int(q["uid"] ?? "10") ?? 10, detail: true)
            bodyRequests += 1
            if ProcessInfo.processInfo.arguments.contains("--body-unavailable") && bodyRequests == 1 {
                body["has_html"] = false
                body["body_error"] = "We couldn’t load this message. Try again."
            }
            if ProcessInfo.processInfo.arguments.contains("--image-policy") {
                body["image_policy"] = imagePolicy()
            }
            result = body
        case "/api/image-trust":
            imageSenderTrusted = body?["trusted"] as? Bool ?? false
            result = imagePolicy()
        case "/api/body", "/api/draft-body": return Data(html.utf8)
        case "/api/folders": result = [["name": "Archive", "label": "Archive", "type": "archive"], ["name": "Projects", "label": "Projects", "type": "other"]]
        case "/api/drafts":
            var draft = message(20); draft["draft"] = true; draft["folder"] = "Drafts"; draft["subject"] = "Re: A good place to pick this up"
            var drafts = [draft]
            if ProcessInfo.processInfo.arguments.contains("--missing-draft") {
                var current = draft; current["uid"] = 21; current["subject"] = "A current draft to review"
                drafts.append(current)
            }
            result = sent ? [] : drafts.filter { !missingDrafts.contains($0["uid"] as! Int) }
        case "/api/action":
            let uid = body?["uid"] as? Int ?? 0
            switch body?["action"] as? String {
            case "archive", "move", "trash": archived.insert(uid)
            case "star": starred.insert(uid)
            case "unstar": starred.remove(uid)
            case "read": read.insert(uid)
            case "unread": read.remove(uid)
            default: break
            }
        case "/api/draft":
            saved = true
            result = ["ok": true, "draft": ["account": "work", "folder": "Drafts", "uid": 20]]
        case "/api/draft-preview":
            let uid = Int(q["uid"] ?? "20") ?? 20
            draftPreviewRequests += 1
            if ProcessInfo.processInfo.arguments.contains("--missing-draft") && uid == 20 {
                missingDrafts.insert(uid); throw MailError.draftUnavailable
            }
            if ProcessInfo.processInfo.arguments.contains("--draft-preview-unavailable") && draftPreviewRequests == 1 { throw URLError(.timedOut) }
            result = ["account_id": "work", "folder": "Drafts", "uid": uid, "from": ["name": "Chris", "email": "me@example.com"], "to": [["email": "jordan@example.org"]], "cc": [], "bcc": [["email": "copy@example.org"]], "subject": "Re: A good place to pick this up", "plain_text": "Thanks, Tuesday works.", "attachments": [], "sendable": true, "digest": String(repeating: "a", count: 64)]
        case "/api/send":
            if ProcessInfo.processInfo.arguments.contains("--stale-draft") { throw MailError.server("The draft changed after you reviewed it. Reopen it and review again.") }
            sent = true; result = ["ok": true, "draft_removed": true]
        case "/api/scheduled": result = scheduled
        case "/api/schedule":
            if ProcessInfo.processInfo.arguments.contains("--stale-draft") { throw MailError.server("The draft changed after you reviewed it. Reopen it and review again.") }
            let row: [String: Any] = ["id": 1, "account": "work", "folder": "Drafts", "uid": 20,
                "send_at": body?["send_at"] as? String ?? "", "subject": "Re: A good place to pick this up",
                "recipients": "jordan@example.org", "status": "pending"]
            scheduled = [row]; result = row
        case "/api/scheduled/cancel":
            if !scheduled.isEmpty { scheduled[0]["status"] = "cancelled" }
        case "/api/chat/start": chatStarts += 1; revision = 0; result = ["ok": true, "chat": 1]
        case "/api/chat/events":
            if q["after"] == "0" { result = ["chat": 1, "seq": 1, "alive": true, "events": [["event": ["type": "ready"]]]] }
            else if revision == 1 {
                revision = 2
                if ProcessInfo.processInfo.arguments.contains("--chat-rich") {
                    result = ["chat": 1, "seq": 3, "alive": true, "events": [
                        ["event": ["type": "tool_use", "id": "search-1", "name": "search_email", "input": ["query": "Jordan"]]],
                        ["event": ["type": "tool_result", "tool_use_id": "search-1", "text": "Two matching messages"]],
                        ["event": ["type": "assistant_text", "text": "## Next steps\n\nHere’s the **short version**:\n\n- Reply to Jordan\n- Pick **Tuesday** or Thursday\n\n> Nothing is sent until you review it.\n\n```text\nTuesday works well.\n```"]],
                        ["event": ["type": "turn_done"]]]]
                } else {
                    result = ["chat": 1, "seq": 3, "alive": true, "events": [["event": ["type": "assistant_text", "text": "Jordan is asking to meet next week. Tuesday or Thursday works for them."]], ["event": ["type": "permission_request", "request_id": "permission-1", "tool_name": "compose_draft", "display_name": "Write a reply", "input": ["to": ["jordan@example.org"]], "rememberable": true, "context": ["account": "me@example.com"]]]]]
                }
            } else if revision == 3 {
                revision = 4
                result = ["chat": 1, "seq": 4, "alive": true, "events": [["event": ["type": "turn_done"]]]]
            } else { result = ["chat": 1, "seq": revision >= 4 ? 4 : revision == 2 ? 3 : 1, "alive": true, "events": []] }
        case "/api/chat/send": lastChatText = body?["text"] as? String ?? ""; revision = 1
        case "/api/chat/interrupt": revision = 3
        case "/api/claude-thread": result = ["ok": true, "threadId": "fixture-thread"]
        default: break
        }
        return try JSONSerialization.data(withJSONObject: result)
    }
}
#endif
