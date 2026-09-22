/// Extract the first <addr-spec> Message-ID from a raw header value, returning
/// it normalized with surrounding angle brackets.
///
/// Accepts both shapes:
///  * IMAP raw header: bracketed list separated by whitespace or commas
///    (e.g. `<a@x> <b@y>` or `<a@x>, <b@y>`).
///  * mail-parser normalized form: brackets stripped, e.g. `a@x` or
///    `a@x, b@y`. The mbox importer goes through mail-parser, so this branch
///    is required to keep imported messages threading correctly.
///
/// Returns None when no Message-ID-shaped token is present.
pub fn parse_first_message_id(raw: &str) -> Option<String> {
    let raw = raw.trim();
    if raw.is_empty() {
        return None;
    }

    // 1) Bracketed: scan for the first non-empty <...> pair. `<>` alone
    //    is malformed; advance past it and keep scanning.
    let mut search = raw;
    while let Some(start) = search.find('<') {
        let after = &search[start + 1..];
        match after.find('>') {
            Some(end) => {
                let inner = after[..end].trim();
                if !inner.is_empty() {
                    return Some(format!("<{}>", inner));
                }
                search = &after[end + 1..];
            }
            None => break,
        }
    }

    // 2) Unbracketed: take the first whitespace/comma/semicolon-separated
    //    token that contains '@' (smallest signal an addr-spec is present).
    raw.split(|c: char| c == ',' || c == ';' || c.is_whitespace())
        .map(str::trim)
        .find(|s| !s.is_empty() && s.contains('@'))
        .map(|s| format!("<{}>", s))
}

/// Undo HTML entity escaping around a Message-ID.
///
/// Agents copy IDs out of rendered/sanitized surfaces, and at least one live
/// draft ([Gmail]/Drafts uid 578) carried
/// `&lt;A7EA3536-…@harborline.example&gt;` in both `In-Reply-To` and `References`
/// — a third spelling that matched nothing and produced an unthreaded reply.
/// Only applied when the escapes are actually present, so a legitimate `&`
/// inside an addr-spec (`a&b@host` is valid atext) is left alone.
fn unescape_bracket_entities(raw: &str) -> String {
    raw.replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&amp;", "&")
}

fn has_bracket_entities(raw: &str) -> bool {
    raw.contains("&lt;") || raw.contains("&gt;")
}

/// Canonical bracketed form of a single Message-ID, accepting every spelling
/// that reaches us: bracketed `<id@host>`, bare `id@host` (mail-parser strips
/// the brackets, so `draft_local` and the mbox importer store this shape), and
/// HTML-escaped `&lt;id@host&gt;`.
///
/// Returns None when no Message-ID-shaped token is present.
pub fn normalize_message_id(raw: &str) -> Option<String> {
    if has_bracket_entities(raw) {
        parse_first_message_id(&unescape_bracket_entities(raw))
    } else {
        parse_first_message_id(raw)
    }
}

/// Both spellings a Message-ID may be stored under, as
/// `("<id@host>", "id@host")`.
///
/// `messages.message_id` holds both (29k bracketed rows from the IMAP sync,
/// 16 bare rows minted by `draft_local`), so every lookup keyed on that column
/// must bind both variants — see gotcha #30.
pub fn message_id_match_variants(raw: &str) -> Option<(String, String)> {
    let bracketed = normalize_message_id(raw)?;
    // `parse_first_message_id` always returns `<`…`>` with a non-empty inner,
    // so trimming one byte from each end is safe.
    let bare = bracketed[1..bracketed.len() - 1].to_string();
    Some((bracketed, bare))
}

/// Normalize a whole `References` chain to space-separated bracketed IDs.
///
/// Space separation matters: `mail_builder`'s `Raw::write_header` folds at
/// whitespace past 76 bytes, so a normalized chain folds legally instead of
/// emitting one over-long header line.
///
/// Unparseable tokens are dropped — the live DB really does contain
/// `=?UTF-8?Q?<hkuwwjq++…?=` in `reference_ids`, and emitting that verbatim
/// corrupts the header for every downstream client. Consecutive duplicates
/// collapse; non-adjacent repeats are kept because chain order is meaningful.
pub fn normalize_reference_chain(raw: &str) -> String {
    let source = if has_bracket_entities(raw) {
        unescape_bracket_entities(raw)
    } else {
        raw.to_string()
    };

    let mut out: Vec<String> = Vec::new();
    let push = |id: String, out: &mut Vec<String>| {
        if out.last().is_some_and(|prev| *prev == id) {
            return;
        }
        out.push(id);
    };

    for token in source.split(|c: char| c == ',' || c == ';' || c.is_whitespace()) {
        let token = token.trim();
        if token.is_empty() {
            continue;
        }
        // A single token can hold several concatenated ids (`<a@x><b@y>`);
        // take them all rather than only the first.
        let mut found_bracketed = false;
        let mut search = token;
        while let Some(start) = search.find('<') {
            let after = &search[start + 1..];
            let Some(end) = after.find('>') else { break };
            let inner = after[..end].trim();
            if !inner.is_empty() {
                found_bracketed = true;
                push(format!("<{}>", inner), &mut out);
            }
            search = &after[end + 1..];
        }
        if !found_bracketed {
            if let Some(id) = parse_first_message_id(token) {
                push(id, &mut out);
            }
        }
    }

    out.join(" ")
}

/// Pick a stable per-account thread root key for a message.
///
/// Order: oldest reference (first Message-ID in References) →
/// `In-Reply-To` → `Message-ID` → folder/uid fallback.
///
/// The fallback key embeds the folder so two messages in different folders
/// of the same account don't accidentally collide on the IMAP UID space
/// (UIDs are folder-scoped).
/// Reply/forward prefixes, including the ones non-English clients emit.
const SUBJECT_PREFIXES: &[&str] = &["re", "fwd", "fw", "aw", "wg", "sv", "vs", "antw", "rif"];

/// Subjects too generic to be evidence that two messages belong together.
/// Threading on these would merge unrelated conversations with the same
/// pleasantry for a subject.
const GENERIC_SUBJECTS: &[&str] = &[
    "hi", "hey", "hello", "thanks", "thank you", "question", "quick question",
    "follow up", "following up", "checking in", "update", "updates", "meeting",
    "call", "intro", "introduction", "invoice", "receipt", "reminder", "notes",
    "info", "information", "request", "inquiry", "test", "no subject",
];

/// Normalize a subject for use as a *fallback* thread key.
///
/// Returns `None` when the subject cannot safely identify a conversation —
/// empty, generic, or too short. `None` means "do not thread on this", and
/// callers must treat it as a refusal rather than as an empty key, or every
/// blank-subject message in the mailbox collapses into one thread.
///
/// This is only ever a fallback. A message carrying `References` or
/// `In-Reply-To` is threaded by those, always: a subject match is a guess, and
/// a header is a fact.
pub fn normalize_subject_for_threading(subject: &str) -> Option<String> {
    let s = strip_reply_prefixes(subject);

    let collapsed = s
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase();

    if collapsed.len() < 8 || GENERIC_SUBJECTS.contains(&collapsed.as_str()) {
        return None;
    }
    Some(collapsed)
}

/// Does this subject announce itself as a reply or forward?
///
/// This is the gate on subject-fallback threading, and it is what keeps the
/// fallback from turning recurring notifications into conversations. Without
/// it, 379 identically-titled Synology container alerts become one 379-message
/// "thread" — measured, not hypothetical. A repeated subject means a repeated
/// notification; a repeated subject with `Re:` in front means someone answered.
pub fn has_reply_prefix(subject: &str) -> bool {
    strip_reply_prefixes(subject).len() != subject.trim().len()
}

fn strip_reply_prefixes(subject: &str) -> &str {
    let mut s = subject.trim();

    // Strip any stack of reply/forward prefixes: "Re: Fwd: Re: Scope" → "Scope".
    // Bounded rather than `loop`ed so a pathological subject can't spin.
    for _ in 0..8 {
        let lower = s.to_ascii_lowercase();
        let Some(stripped) = SUBJECT_PREFIXES.iter().find_map(|p| {
            // Accept "re:", "re :", and the "re[2]:" some clients emit.
            let rest = lower.strip_prefix(p)?;
            let rest = rest.trim_start();
            let rest = match rest.strip_prefix('[') {
                Some(after) => after.split_once(']').map(|(_, r)| r.trim_start())?,
                None => rest,
            };
            rest.strip_prefix(':').map(|r| s.len() - r.len())
        }) else {
            break;
        };
        s = s[stripped..].trim_start();
    }
    s
}

pub fn compute_thread_root_id(
    message_id: Option<&str>,
    in_reply_to: Option<&str>,
    references_raw: Option<&str>,
    folder_name: &str,
    uid: u32,
) -> String {
    references_raw
        .and_then(parse_first_message_id)
        .or_else(|| in_reply_to.and_then(parse_first_message_id))
        .or_else(|| message_id.and_then(parse_first_message_id))
        .unwrap_or_else(|| format!("uid:{}:{}", folder_name, uid))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subject_normalization_strips_any_stack_of_prefixes() {
        let canonical = normalize_subject_for_threading(
            "Northwind Company / CX Ventures Meeting Follow Up",
        );
        assert!(canonical.is_some());
        for variant in [
            "Re: Northwind Company / CX Ventures Meeting Follow Up",
            "RE: Fwd: Northwind Company / CX Ventures Meeting Follow Up",
            "Fwd: Re: Re: Northwind Company / CX Ventures Meeting Follow Up",
            "re[2]: Northwind Company / CX Ventures Meeting Follow Up",
            "  Re:   Northwind Company / CX  Ventures Meeting Follow Up  ",
            "AW: Northwind Company / CX Ventures Meeting Follow Up",
        ] {
            assert_eq!(
                normalize_subject_for_threading(variant),
                canonical,
                "{variant:?} must normalize to the same key"
            );
        }
    }

    /// Refusing is not the same as returning an empty key: threading on "" would
    /// collapse every blank-subject message in the mailbox into one conversation.
    #[test]
    fn subject_normalization_refuses_subjects_that_prove_nothing() {
        for weak in ["", "   ", "Re:", "Hi", "hello", "Thanks", "Quick question", "update", "Re: Hi"] {
            assert_eq!(
                normalize_subject_for_threading(weak),
                None,
                "{weak:?} is not specific enough to identify a thread"
            );
        }
    }

    #[test]
    fn subject_normalization_keeps_a_real_subject_that_merely_starts_with_re() {
        // "Retainer" starts with "re" but has no colon — not a prefix.
        assert_eq!(
            normalize_subject_for_threading("Retainer proposal for Q3"),
            Some("retainer proposal for q3".to_string())
        );
    }

    #[test]
    fn parses_single_id() {
        assert_eq!(
            parse_first_message_id("<abc@host>"),
            Some("<abc@host>".to_string())
        );
    }

    #[test]
    fn parses_first_of_whitespace_separated_list() {
        assert_eq!(
            parse_first_message_id("<root@x> <reply1@y> <reply2@z>"),
            Some("<root@x>".to_string())
        );
    }

    #[test]
    fn parses_first_of_comma_separated_list() {
        assert_eq!(
            parse_first_message_id("<root@x>, <reply1@y>"),
            Some("<root@x>".to_string())
        );
    }

    #[test]
    fn handles_inner_whitespace() {
        assert_eq!(
            parse_first_message_id("<  abc@host  >"),
            Some("<abc@host>".to_string())
        );
    }

    #[test]
    fn parses_unbracketed_single_id() {
        // mail-parser strips brackets; mbox-imported rows arrive in this shape.
        assert_eq!(
            parse_first_message_id("abc@host"),
            Some("<abc@host>".to_string())
        );
    }

    #[test]
    fn parses_first_unbracketed_in_list() {
        assert_eq!(
            parse_first_message_id("root@x b@y c@z"),
            Some("<root@x>".to_string())
        );
        assert_eq!(
            parse_first_message_id("root@x, b@y"),
            Some("<root@x>".to_string())
        );
    }

    #[test]
    fn returns_none_for_no_addr_spec() {
        assert_eq!(parse_first_message_id("not an id"), None);
    }

    #[test]
    fn returns_none_for_empty_brackets() {
        // <> alone is malformed and contains no addr-spec elsewhere.
        assert_eq!(parse_first_message_id("<>"), None);
    }

    #[test]
    fn empty_brackets_then_unbracketed_id_still_resolves() {
        assert_eq!(
            parse_first_message_id("<> abc@host"),
            Some("<abc@host>".to_string())
        );
    }

    // ─── Normalizers (gotcha #30) ─────────────────────────────

    #[test]
    fn normalize_message_id_accepts_all_three_spellings() {
        let want = Some("<a@x>".to_string());
        assert_eq!(normalize_message_id("<a@x>"), want);
        assert_eq!(normalize_message_id("a@x"), want);
        // The live [Gmail]/Drafts uid 578 spelling.
        assert_eq!(normalize_message_id("&lt;a@x&gt;"), want);
        assert_eq!(normalize_message_id("   <a@x>  "), want);
        assert_eq!(normalize_message_id("garbage"), None);
    }

    #[test]
    fn normalize_message_id_keeps_legitimate_ampersand() {
        // `&` is valid atext — only bracket entities trigger unescaping.
        assert_eq!(normalize_message_id("<a&b@x>"), Some("<a&b@x>".to_string()));
    }

    #[test]
    fn match_variants_yield_same_pair_for_both_spellings() {
        let from_bracketed = message_id_match_variants("<abc@host>").unwrap();
        let from_bare = message_id_match_variants("abc@host").unwrap();
        let from_escaped = message_id_match_variants("&lt;abc@host&gt;").unwrap();
        assert_eq!(
            from_bracketed,
            ("<abc@host>".to_string(), "abc@host".to_string())
        );
        assert_eq!(from_bracketed, from_bare);
        assert_eq!(from_bracketed, from_escaped);
        assert!(message_id_match_variants("not an id").is_none());
    }

    #[test]
    fn reference_chain_brackets_bare_tokens() {
        // The `draft_local` shape: mail-parser output joined with spaces.
        assert_eq!(
            normalize_reference_chain("root@x parent@y"),
            "<root@x> <parent@y>"
        );
    }

    #[test]
    fn reference_chain_is_idempotent_on_bracketed_input() {
        let chain = "<root@x> <parent@y> <self@z>";
        assert_eq!(normalize_reference_chain(chain), chain);
        assert_eq!(
            normalize_reference_chain(&normalize_reference_chain(chain)),
            chain
        );
    }

    #[test]
    fn reference_chain_drops_the_real_rfc2047_garbage() {
        // Verbatim from messages.reference_ids, [Gmail]/All Mail uid 52625.
        let garbage = "=?UTF-8?Q?<hkuwwjq++pguifmoxmymj=C3=A9=C3=A9&&-=5F=5F=3D=5F=5FB?=";
        assert_eq!(normalize_reference_chain(garbage), "");
        // …and doesn't take a valid neighbour down with it.
        assert_eq!(
            normalize_reference_chain(&format!("{} <ok@x>", garbage)),
            "<ok@x>"
        );
    }

    #[test]
    fn reference_chain_handles_commas_entities_and_concatenation() {
        assert_eq!(normalize_reference_chain("<a@x>, <b@y>"), "<a@x> <b@y>");
        assert_eq!(
            normalize_reference_chain("&lt;a@x&gt; &lt;b@y&gt;"),
            "<a@x> <b@y>"
        );
        assert_eq!(normalize_reference_chain("<a@x><b@y>"), "<a@x> <b@y>");
    }

    #[test]
    fn reference_chain_collapses_consecutive_duplicates_only() {
        assert_eq!(
            normalize_reference_chain("<a@x> <a@x> <b@y>"),
            "<a@x> <b@y>"
        );
        // Non-adjacent repeat is meaningful chain order — keep it.
        assert_eq!(
            normalize_reference_chain("<a@x> <b@y> <a@x>"),
            "<a@x> <b@y> <a@x>"
        );
        assert_eq!(normalize_reference_chain(""), "");
    }

    #[test]
    fn root_prefers_first_reference() {
        let root = compute_thread_root_id(
            Some("<self@x>"),
            Some("<parent@y>"),
            Some("<root@a> <parent@y>"),
            "INBOX",
            42,
        );
        assert_eq!(root, "<root@a>");
    }

    #[test]
    fn root_falls_back_to_in_reply_to() {
        let root = compute_thread_root_id(Some("<self@x>"), Some("<parent@y>"), None, "INBOX", 42);
        assert_eq!(root, "<parent@y>");
    }

    #[test]
    fn root_falls_back_to_message_id() {
        let root = compute_thread_root_id(Some("<self@x>"), None, None, "INBOX", 42);
        assert_eq!(root, "<self@x>");
    }

    #[test]
    fn root_falls_back_to_folder_uid() {
        let root = compute_thread_root_id(None, None, None, "[Gmail]/Sent Mail", 42);
        assert_eq!(root, "uid:[Gmail]/Sent Mail:42");
    }
}
