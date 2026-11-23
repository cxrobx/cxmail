#if DEBUG
import Foundation
import UIKit

/// Fiction-only, in-process transport. UI tests can never touch a real mailbox.
@MainActor enum Fixtures {
    static var draftLinkDelay: Duration = .zero
    static var draftLinkUnavailable = false
    static var draftLinkUID = 20
    static var expireSearchCursor = false
    static var searchBusyCount = 0
    static var searchDelay: Duration = .zero
    static var archived = Set<Int>()
    static var starred = Set<Int>()
    static var read = Set<Int>()
    static var notify = Set<Int>()
    static var saved = false
    static var editedSubject: String?
    static var sent = false
    static var revision = 0
    static var bodyRequests = 0
    static var chatStarts = 0
    static var lastChatText = ""
    static var lastChatImages = 0
    static var chatSentAt = Date.distantPast
    static let newerRevisionText = "Thanks, Tuesday works. Added on the Mac."
    static var lastDraftPhotos = 0
    /// Same rule as the mini: base64 JPEG only from the phone.
    static func validPhotos(_ value: Any?) throws -> Int {
        guard let list = value as? [String] else { return 0 }
        for item in list {
            guard let data = Data(base64Encoded: item), data.starts(with: [0xFF, 0xD8, 0xFF]) else {
                throw MailError.server("photos must be JPEG, PNG, GIF or WebP")
            }
        }
        return list.count
    }
    /// A fiction-only clipboard image so UI tests can exercise Paste.
    static func seedPasteboardPhoto() {
        let image = UIGraphicsImageRenderer(size: CGSize(width: 320, height: 240)).image { ctx in
            UIColor.systemTeal.setFill(); ctx.fill(CGRect(x: 0, y: 0, width: 320, height: 240))
            UIColor.white.setFill(); ctx.fill(CGRect(x: 100, y: 70, width: 120, height: 100))
        }
        UIPasteboard.general.image = image
    }
    static var scheduled: [[String: Any]] = []
    static var missingDrafts = Set<Int>()
    static var draftPreviewRequests = 0
    static func message(_ uid: Int = 10, detail: Bool = false) -> [String: Any] {
        var value: [String: Any] = ["account": "work", "folder": "INBOX", "uid": uid,
            "subject": uid == 10 ? "A good place to pick this up" : "The next steps for our project",
            "from_name": uid == 10 ? "Jordan Lee" : "Alex Morgan", "from_email": "jordan@example.org",
            "date": "2026-11-14T17:30:00+00:00", "snippet": "Thanks for the thoughtful notes. Here’s what I’m thinking for next week.",
            "read": read.contains(uid), "starred": starred.contains(uid), "notify": notify.contains(uid), "attachments": true, "category": "primary", "recipients": ["Chris"]]
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
    static let signature = """
    <div class="email-signature" style="margin-top:16px;font-family:-apple-system,sans-serif">
      <table cellpadding="0" cellspacing="0" style="border-collapse:collapse"><tr>
        <td style="padding-right:16px;vertical-align:middle;border-right:2px solid #ccc">
          <div style="font-size:36px;font-weight:700;letter-spacing:2px; color: #999;line-height:1">BS</div>
          <div style="font-size:9px;letter-spacing:3px; color: #aaa">STUDIO</div>
        </td>
        <td style="padding-left:16px;vertical-align:middle">
          <div style="font-size:16px;font-weight:600; color: #333">Morgan Reese</div>
          <div style="font-size:13px; color: #777">Bluebird Studio</div>
          <div style="margin-top:8px;font-size:12px; color: #888">morgan@example.com · bluebird.example.org</div>
        </td>
      </tr></table>
    </div>
    """
    // Same 600px table class as fixed-width account notices, with fiction only.
    static let fixedWidthHTML = """
    <html><head><meta name="viewport" content="width=device-width,initial-scale=1"></head>
    <body style="font:18px -apple-system;color:#202124;background:white">
    <table width="600" align="center" style="width:600px;min-width:600px;border-collapse:collapse"><tbody>
    <tr><td style="padding:10px"><span style="float:right">Right edge</span><span>Bluebird Studio</span></td></tr>
    <tr><td style="padding:10px"><h2>New sign-in to your account</h2><p>Hello Alex,</p>
    <p>We noticed a new sign-in to your Bluebird Studio account. You can review the complete details below.</p>
    <p style="white-space:nowrap"><strong>IP address: 2001:db8:1234:5678:90ab:cdef:1234:5678</strong></p>
    <p>Time: 15 November 2026, 12:00 UTC</p>
    <p>If this was you, you can ignore this email.</p>
    <p>If you do not recognise this sign-in, <a href="https://example.org/account">review your account</a> or contact our support team.</p>
    <h2>Need support?</h2><p>Find answers at <a href="https://example.org/help">https://example.org/help</a>.</p>
    <p>Keep this complete account notice for your records.</p><p>Bottom of the complete email</p>
    </td></tr></tbody></table></body></html>
    """
    static func response(_ path: String, query: [URLQueryItem], body: [String: Any]?) throws -> Data {
        let q = Dictionary(query.map { ($0.name, $0.value ?? "") }, uniquingKeysWith: { _, n in n })
        var result: Any = ["ok": true]
        switch path {
        case "/api/accounts": result = [["id": "work", "email": "me@example.com", "name": "Work", "hidden": false], ["id": "personal", "email": "personal@example.com", "name": "Personal", "hidden": false]]
        case "/api/compose-context":
            result = ["from": q["account"] == "personal" ? "personal@example.com" : "me@example.com",
                      "html": q["account"] == "personal" ? NSNull() as Any : signature]
        case "/api/messages":
            if ProcessInfo.processInfo.arguments.contains("--offline") { throw URLError(.notConnectedToInternet) }
            if ProcessInfo.processInfo.arguments.contains("--empty") || (q["q"] != nil && q["q"] != "project") { result = [[String: Any]]() }
            else { result = (10...17).map { message($0) }.filter { !archived.contains($0["uid"] as! Int) && (q["view"] != "unread" || !($0["read"] as! Bool)) && (q["view"] != "starred" || ($0["starred"] as! Bool)) } }
        case "/api/search":
            if q["cursor"] != nil && expireSearchCursor { expireSearchCursor = false; throw MailError.searchExpired }
            if q["mode"] != "keyword" && searchBusyCount > 0 { searchBusyCount -= 1; throw MailError.searchBusy }
            let matches = ["project", "project notes"].contains(q["q"] ?? "")
            result = ["results": matches ? [message(10), message(11)] : [], "semantic_status": q["mode"] == "keyword" ? "keyword" : "ready", "next_cursor": expireSearchCursor ? "fixture:50" : NSNull()]
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
        case "/api/notifications/thread":
            let uid = body?["uid"] as? Int ?? 0
            if body?["enabled"] as? Bool == true { notify.insert(uid) } else { notify.remove(uid) }
            result = ["ok": true, "notify": notify.contains(uid)]
        case "/api/image-trust":
            imageSenderTrusted = body?["trusted"] as? Bool ?? false
            result = imagePolicy()
        case "/api/body", "/api/draft-body": return Data((ProcessInfo.processInfo.arguments.contains("--fixed-width-email") ? fixedWidthHTML : html).utf8)
        case "/api/folders": result = [["name": "Archive", "label": "Archive", "type": "archive"], ["name": "Projects", "label": "Projects", "type": "other"]]
        case "/api/draft-link":
            if draftLinkUnavailable || ProcessInfo.processInfo.arguments.contains("--draft-link-unavailable") { throw URLError(.notConnectedToInternet) }
            result = ["account": "work", "folder": "Drafts", "uid": ProcessInfo.processInfo.arguments.contains("--newer-revision") ? 21 : draftLinkUID]
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
            lastDraftPhotos = try validPhotos(body?["photos"])
            if ProcessInfo.processInfo.arguments.contains("--paste-photo") && lastDraftPhotos != 1 {
                throw MailError.server("Expected exactly one pasted photo with this draft (got \(lastDraftPhotos), files \((body?["files"] as? [Any])?.count ?? 0)).")
            }
            saved = true
            result = ["ok": true, "draft": ["account": "work", "folder": "Drafts", "uid": 20]]
        case "/api/draft-preview":
            let uid = Int(q["uid"] ?? "20") ?? 20
            draftPreviewRequests += 1
            if ProcessInfo.processInfo.arguments.contains("--missing-draft") && uid == 20 {
                missingDrafts.insert(uid); throw MailError.draftUnavailable
            }
            if ProcessInfo.processInfo.arguments.contains("--draft-preview-unavailable") && draftPreviewRequests == 1 { throw URLError(.timedOut) }
            result = ["account_id": "work", "folder": "Drafts", "uid": uid, "from": ["name": "Chris", "email": "me@example.com"], "to": [["email": "jordan@example.org"]], "cc": [], "bcc": [["email": "copy@example.org"]], "subject": uid == 21 ? (editedSubject ?? "Re: A good place to pick this up") : "Re: A good place to pick this up", "plain_text": "Thanks, Tuesday works.", "edit_text": ProcessInfo.processInfo.arguments.contains("--newer-revision") && uid >= 21 ? newerRevisionText : "Thanks, Tuesday works.", "draft_id": "0c5c3b0e-1d2f-4a3b-9c4d-5e6f7a8b9c0d", "attachments": ProcessInfo.processInfo.arguments.contains("--draft-attachment-fixture") ? ["Study-planner.docx"] : [], "sendable": true, "digest": String(repeating: "a", count: 64)]
        case "/api/draft-edit":
            if ProcessInfo.processInfo.arguments.contains("--newer-revision") {
                // Revision 20 was replaced elsewhere by 21, whose text differs.
                if body?["uid"] as? Int == 20 { throw MailError.draftUnavailable }
                guard body?["uid"] as? Int == 21, body?["body"] as? String == newerRevisionText else {
                    throw MailError.server("Expected the latest text on revision 21.")
                }
                lastDraftPhotos = try validPhotos(body?["photos"])
                if ProcessInfo.processInfo.arguments.contains("--paste-photo") && lastDraftPhotos != 1 {
                    throw MailError.server("Expected exactly one pasted photo with this draft (got \(lastDraftPhotos), files \((body?["files"] as? [Any])?.count ?? 0)).")
                }
                result = ["ok": true, "draft": ["account": "work", "folder": "Drafts", "uid": 22]]
                break
            }
            guard body?["uid"] as? Int == 20, body?["digest"] as? String == String(repeating: "a", count: 64),
                  body?["bcc"] as? [String] == ["copy@example.org"]
            else { throw MailError.reviewExpired }
            editedSubject = body?["subject"] as? String
            lastDraftPhotos = try validPhotos(body?["photos"])
            if ProcessInfo.processInfo.arguments.contains("--paste-photo") && lastDraftPhotos != 1 {
                throw MailError.server("Expected exactly one pasted photo with this draft (got \(lastDraftPhotos), files \((body?["files"] as? [Any])?.count ?? 0)).")
            }
            result = ["ok": true, "draft": ["account": "work", "folder": "Drafts", "uid": 21]]
        case "/api/draft-attachment":
            guard q["uid"] == "20", q["index"] == "0", q["digest"] == String(repeating: "a", count: 64)
            else { throw MailError.reviewExpired }
            if ProcessInfo.processInfo.arguments.contains("--attachment-review-expired") { throw MailError.reviewExpired }
            if q["render"] == "word", ProcessInfo.processInfo.arguments.contains("--word-page-fixture") {
                return UIGraphicsPDFRenderer(bounds: CGRect(x: 0, y: 0, width: 612, height: 792)).pdfData { output in
                    output.beginPage()
                    ("Study planner QA sample" as NSString).draw(at: CGPoint(x: 60, y: 60), withAttributes: [
                        .font: UIFont.systemFont(ofSize: 24), .foregroundColor: UIColor.black,
                    ])
                }
            }
            return Data(base64Encoded: "UEsDBBQAAAAIAFl7cV10JJxTuwAAAD4BAAATAAAAW0NvbnRlbnRfVHlwZXNdLnhtbJWQuQ7CMAyGX6XKiqgRAwNquwArMPACVuq2EbkUuxxvT8o1sDHa//FZrk73SFzcnPVcq0EkrgFYD+SQyxDJZ6ULyaHkMfUQUZ+xJ1guFivQwQt5mcvUoZpqSx2OVordLa/ZBF+rRJZVsXkZJ1atMEZrNErW4eLbH8r8TShz8unhwUSeZYOCpjpcKCXTUnHEJHt0uQ6uIbXQBj26jCgn41+80HVG0zc/tcUUNDEb3ztbfhWHxn/ugOfbmgdQSwMEFAAAAAgAWXtxXWF7L0OJAAAA8gAAAAsAAABfcmVscy8ucmVsc43POw4CIRAG4KsQDrCzWlgYoLLZ1ngBAsMjLo8MGPX2UlisxsJy5p98f0accdU9ltxCrI090pqb5KH3egRoJmDSbSoV80hcoaT7GMlD1eaqPcJ+ng9AW4MrsTXZYiWnxe44uzwr/mMX56LBUzG3hLn/qPi6GLImj13yeyEL9r2eBstBCfh4Ub0AUEsDBBQAAAAIAFl7cV19AY7jgQAAAKkAAAARAAAAd29yZC9kb2N1bWVudC54bWxFzkEKwyAQBdCrSA7QCV10IVboEUpPYNQmAWccRoPN7RvTRTfvM3z4jGk6ZL9hpKo+mKjodh+WWlkDFL9EdOWSOdLRvbOgq8cpM7QsgSX7WMpKMya4juMN0K00WNP0lMPekzvSqfZVt7ArTo4oino+VHHIKRroZVdO+fQ3AP/n7BdQSwECFAMUAAAACABZe3FddCScU7sAAAA+AQAAEwAAAAAAAAAAAAAAgAEAAAAAW0NvbnRlbnRfVHlwZXNdLnhtbFBLAQIUAxQAAAAIAFl7cV1hey9DiQAAAPIAAAALAAAAAAAAAAAAAACAAewAAABfcmVscy8ucmVsc1BLAQIUAxQAAAAIAFl7cV19AY7jgQAAAKkAAAARAAAAAAAAAAAAAACAAZ4BAAB3b3JkL2RvY3VtZW50LnhtbFBLBQYAAAAAAwADALkAAABOAgAAAAA=")!
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
            else if revision == 1, ProcessInfo.processInfo.arguments.contains("--paste-photo"),
                    Date().timeIntervalSince(chatSentAt) < 3 {
                // Hold the reply so the test can see the sent photo before it scrolls away.
                result = ["chat": 1, "seq": 1, "alive": true, "events": []]
            }
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
        case "/api/chat/send":
            lastChatText = body?["text"] as? String ?? ""; revision = 1; chatSentAt = Date()
            lastChatImages = try validPhotos(body?["images"])
            if ProcessInfo.processInfo.arguments.contains("--paste-photo") && lastChatImages != 1 {
                throw MailError.server("Expected exactly one pasted photo with this message.")
            }
        case "/api/chat/interrupt": revision = 3
        case "/api/claude-thread":
            if ProcessInfo.processInfo.arguments.contains("--session-fixture") {
                guard body?["account"] as? String == "work", body?["folder"] as? String == "INBOX",
                      body?["uid"] as? Int == 10, body?["prompt"] as? String == "Summarize next steps" else {
                    throw MailError.server("The session request did not match the selected email and message.")
                }
            }
            result = ["ok": true, "threadId": "fixture-thread"]
        default: break
        }
        return try JSONSerialization.data(withJSONObject: result)
    }
}
#endif
