//! House-style dash normalization — the enforcement half of the pinned rule
//! "Never use em-dashes".
//!
//! ## Why this exists as code rather than as prompt wording
//!
//! The rule was already pinned (`db::voice_pinned_rules`, account scope) and
//! already surfaced on every draft by `mcp::server::pinned_rule_advisory`, and
//! em-dashes still went out. Two reasons, and neither is fixable by rewording:
//!
//! 1. **The advisory arrives after the write.** It rides in the *tool result*
//!    of `compose_draft`/`edit_draft`, so the draft is on IMAP before the model
//!    is reminded of the rule. It buys a follow-up `edit_draft`, not compliance.
//! 2. **The derived voice profile argues the other way.** Every stored profile
//!    described the user's real past habit — `"punctuation_style": "Uses
//!    em-dashes, parentheses, bullets…"` — and `ai::build_voice_context_full`
//!    feeds the *recipient* profile in as the style to match. The model reads
//!    "never use em-dashes" and "uses em-dashes" in one context window.
//!
//! A pinned rule states an absolute; only a deterministic pass can keep one.
//! This module is that pass. It runs at the obligatory write boundary, which
//! also means it catches text the model never authored — a paste, or (the case
//! that prompted it) an external voice-rewrite service handing back prose with
//! em-dashes reinserted.
//!
//! ## What it will not do
//!
//! - **Never a hyphen.** A hyphen stand-in is banned by the rule itself, and it
//!   is the substitution every naive "de-AI" script reaches for.
//! - **Never inside a tag.** `layout: "rich"` bodies are hand-built HTML; a
//!   blind `replace('—', ", ")` would corrupt `href`s, `style` attributes and
//!   `<style>`/`<script>` bodies. The scanner copies tag interiors verbatim.
//! - **Never inside a URL.** A dash inside `https://…` is part of the address.
//! - **Never the quoted original.** Callers normalize the *authored* body only,
//!   before `format_quoted_history` output is appended — that block is a
//!   byte-contract with the frontend (gotcha #30) and it is someone else's
//!   prose besides.
//! - **Never an unspaced en-dash.** `2010–2020` is a range, not a parenthetical
//!   dash. Only the spaced form ` – ` is treated as the banned punctuation use.

use regex::Regex;
use std::sync::OnceLock;

/// EN DASH. Banned only when spaced on both sides (parenthetical use).
const EN_DASH: char = '\u{2013}';

/// Dashes that are never acceptable in outgoing prose, in any spacing.
fn always_banned(c: char) -> bool {
    matches!(
        c,
        '\u{2014}'   // EM DASH
        | '\u{2015}' // HORIZONTAL BAR
        | '\u{2E3A}' // TWO-EM DASH
        | '\u{2E3B}' // THREE-EM DASH
    )
}

/// True if `s` holds any dash this module would rewrite. Used by tests and by
/// callers that want to assert a surface is clean.
pub fn contains_banned_dash(s: &str) -> bool {
    normalize_dashes(s).replaced > 0
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Tok {
    /// Banned regardless of spacing.
    Always,
    /// Banned only when spaced on both sides.
    En,
}

/// HTML spellings. A literal-char replace misses every one of these, and a
/// `layout: "rich"` body written by an agent routinely contains `&mdash;`.
const ENTITIES: &[(&str, Tok)] = &[
    ("&mdash;", Tok::Always),
    ("&#8212;", Tok::Always),
    ("&#x2014;", Tok::Always),
    ("&horbar;", Tok::Always),
    ("&#8213;", Tok::Always),
    ("&#x2015;", Tok::Always),
    ("&ndash;", Tok::En),
    ("&#8211;", Tok::En),
    ("&#x2013;", Tok::En),
];

/// Outcome of a normalization pass.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct DashFix {
    pub text: String,
    /// How many dash runs were rewritten (a run of `———` counts once).
    pub replaced: usize,
    /// Up to three short excerpts of the rewritten text, for reporting back.
    pub samples: Vec<String>,
}

impl DashFix {
    pub fn changed(&self) -> bool {
        self.replaced > 0
    }
}

/// Longest sample list worth putting in a tool result.
const MAX_SAMPLES: usize = 3;
/// Characters of context on each side of a replacement in a sample.
const SAMPLE_RADIUS: usize = 44;

/// Rewrite every banned dash in `input` to house style.
///
/// Idempotent: the output contains no banned dash, so a second pass is a no-op.
/// Char-safe throughout — the scan walks `char`s and never slices by byte
/// offset (gotcha #21b: a length check is not a boundary check).
pub fn normalize_dashes(input: &str) -> DashFix {
    let chars: Vec<char> = input.chars().collect();
    let lower: Vec<char> = chars.iter().map(|c| c.to_ascii_lowercase()).collect();

    let mut out = String::with_capacity(input.len());
    let mut marks: Vec<usize> = Vec::new();
    let mut replaced = 0usize;

    // Tag state. `verbatim` covers <style>/<script> bodies, whose contents are
    // not prose and must survive byte-for-byte.
    let mut in_tag = false;
    let mut pending_verbatim: Option<&'static str> = None;
    let mut verbatim: Option<&'static str> = None;

    let mut i = 0usize;
    while i < chars.len() {
        let c = chars[i];

        if let Some(closing) = verbatim {
            if c == '<' && starts_with_ci(&lower, i, closing) {
                verbatim = None;
            } else {
                out.push(c);
                i += 1;
                continue;
            }
        }

        if in_tag {
            out.push(c);
            if c == '>' {
                in_tag = false;
                verbatim = pending_verbatim.take();
            }
            i += 1;
            continue;
        }

        if c == '<' {
            if starts_with_ci(&lower, i, "<style") {
                pending_verbatim = Some("</style");
            } else if starts_with_ci(&lower, i, "<script") {
                pending_verbatim = Some("</script");
            }
            in_tag = true;
            out.push(c);
            i += 1;
            continue;
        }

        if let Some((len, kind)) = dash_token_at(&chars, i) {
            if banned_at(&chars, i, len, kind) && !in_url_token(&out) {
                // Consume the whole run: dash, then any further dashes with
                // only spaces between them, then the run's trailing spaces.
                // `———` and `— —` are one replacement, not three.
                let mut j = i + len;
                loop {
                    let mut k = j;
                    while k < chars.len() && matches!(chars[k], ' ' | '\t') {
                        k += 1;
                    }
                    match dash_token_at(&chars, k) {
                        Some((l2, k2)) if banned_at(&chars, k, l2, k2) => j = k + l2,
                        _ => break,
                    }
                }
                let mut after = j;
                while after < chars.len() && matches!(chars[after], ' ' | '\t') {
                    after += 1;
                }

                apply_replacement(&mut out, chars.get(after).copied());
                marks.push(out.len());
                replaced += 1;
                i = after;
                continue;
            }
        }

        out.push(c);
        i += 1;
    }

    let samples = marks
        .iter()
        .take(MAX_SAMPLES)
        .map(|&pos| context_window(&out, pos, SAMPLE_RADIUS))
        .collect();

    DashFix {
        text: out,
        replaced,
        samples,
    }
}

/// A dash token at `i`, if any: either one char or one HTML entity.
fn dash_token_at(chars: &[char], i: usize) -> Option<(usize, Tok)> {
    let c = *chars.get(i)?;
    if always_banned(c) {
        return Some((1, Tok::Always));
    }
    if c == EN_DASH {
        return Some((1, Tok::En));
    }
    if c == '&' {
        for (needle, kind) in ENTITIES {
            let n = needle.chars().count();
            if i + n <= chars.len()
                && chars[i..i + n]
                    .iter()
                    .map(|c| c.to_ascii_lowercase())
                    .eq(needle.chars())
            {
                return Some((n, *kind));
            }
        }
    }
    None
}

/// Whether the token at `pos` is actually banned here. The en-dash rule is
/// contextual: spaced is punctuation, unspaced is a range.
fn banned_at(chars: &[char], pos: usize, len: usize, kind: Tok) -> bool {
    match kind {
        Tok::Always => true,
        Tok::En => {
            let before = pos == 0 || chars[pos - 1].is_whitespace();
            let after = chars
                .get(pos + len)
                .map_or(false, |c: &char| c.is_whitespace());
            before && after
        }
    }
}

/// True when the text being written is inside a URL, where a dash is address,
/// not punctuation.
fn in_url_token(out: &str) -> bool {
    let tail = match out.rfind(char::is_whitespace) {
        Some(p) => &out[p..],
        None => out,
    };
    tail.contains("://") || tail.contains("www.")
}

/// Write the replacement for one consumed dash run.
///
/// `next` is the first character after the run and its trailing spaces.
/// Replacement policy, in order:
///
/// | Situation | Result | Example |
/// |---|---|---|
/// | opens a line, follows a tag or an opening delimiter | dropped | `<p>— Chris` → `<p>Chris` |
/// | ends a line, precedes a tag or punctuation | dropped | `…done —\n` → `…done\n` |
/// | digit on both sides | ` to ` | `2010—2020` → `2010 to 2020` |
/// | follows punctuation | one space | `Hi, — how are you` → `Hi, how are you` |
/// | anything else | `, ` | `failures—and more` → `failures, and more` |
fn apply_replacement(out: &mut String, next: Option<char>) {
    let prev = out.chars().rev().find(|c| !matches!(c, ' ' | '\t'));

    let at_open = match prev {
        None => true,
        Some('\n') | Some('\r') => true,
        // A closed tag is a structural boundary: `<p>—` opens a line.
        Some('>') => true,
        Some(c) => matches!(c, '(' | '[' | '{' | '"' | '\'' | '\u{201C}' | '\u{2018}' | '«'),
    };
    if at_open {
        // Leading whitespace here is indentation; leave it alone.
        return;
    }

    let at_close = match next {
        None => true,
        Some('\n') | Some('\r') | Some('<') => true,
        Some(c) => matches!(
            c,
            ')' | ']'
                | '}'
                | '"'
                | '\''
                | '\u{201D}'
                | '\u{2019}'
                | '»'
                | ','
                | '.'
                | ';'
                | ':'
                | '!'
                | '?'
        ),
    };

    trim_trailing_spaces(out);
    if at_close {
        return;
    }

    let prev_c = prev.unwrap_or(' ');
    let next_c = next.unwrap_or(' ');

    if prev_c.is_ascii_digit() && (next_c.is_ascii_digit() || next_c == '$') {
        out.push_str(" to ");
    } else if matches!(prev_c, ',' | ';' | ':' | '.' | '!' | '?') {
        out.push(' ');
    } else {
        out.push_str(", ");
    }
}

fn trim_trailing_spaces(out: &mut String) {
    while out.ends_with(' ') || out.ends_with('\t') {
        out.pop();
    }
}

fn starts_with_ci(lower: &[char], at: usize, needle: &str) -> bool {
    let n = needle.chars().count();
    at + n <= lower.len() && lower[at..at + n].iter().copied().eq(needle.chars())
}

/// A one-line excerpt of `s` centred on byte offset `at`, for reporting a
/// replacement back to the caller. Char-safe.
fn context_window(s: &str, at: usize, radius: usize) -> String {
    let chars: Vec<char> = s.chars().collect();
    let idx = s
        .char_indices()
        .position(|(b, _)| b >= at)
        .unwrap_or(chars.len());
    let start = idx.saturating_sub(radius);
    let end = (idx + radius).min(chars.len());
    let body: String = chars[start..end].iter().collect();
    let body = body.split_whitespace().collect::<Vec<_>>().join(" ");
    format!(
        "{}{}{}",
        if start > 0 { "…" } else { "" },
        body,
        if end < chars.len() { "…" } else { "" }
    )
}

// ─────────────────────────────────────────────────────────────────────────────
// Voice-profile prose
// ─────────────────────────────────────────────────────────────────────────────

fn dash_word_re() -> &'static Regex {
    static RE: OnceLock<Regex> = OnceLock::new();
    RE.get_or_init(|| {
        Regex::new(r"(?i)\b(?:em[-\s]?dash(?:es)?|en[-\s]?dash(?:es)?|m-dash(?:es)?|long dash(?:es)?)\b")
            .expect("valid dash-word regex")
    })
}

fn removal_res() -> &'static Vec<Regex> {
    static RES: OnceLock<Vec<Regex>> = OnceLock::new();
    RES.get_or_init(|| {
        let dw = r"(?:em[-\s]?dash(?:es)?|en[-\s]?dash(?:es)?|m-dash(?:es)?|long dash(?:es)?)";
        [
            // "em-dashes/line breaks" → "line breaks"
            format!(r"(?i)\b{dw}\s*/\s*"),
            format!(r"(?i)\s*/\s*{dw}\b"),
            // "em dashes or parenthetical asides" → "parenthetical asides"
            format!(r"(?i)\b{dw}\s+(?:and|or)\s+"),
            format!(r"(?i)\s+(?:and|or)\s+{dw}\b"),
            // bare mention
            format!(r"(?i)\b{dw}\b"),
        ]
        .iter()
        .map(|p| Regex::new(p).expect("valid dash-removal regex"))
        .collect()
    })
}

/// Words that cannot carry a clause on their own. A segment left holding only
/// these after the dash phrase is removed ("Uses em-dashes" → "Uses") is a
/// fragment, not information, so it is dropped whole.
const FILLER: &[&str] = &[
    "uses", "use", "using", "used", "with", "and", "or", "occasional",
    "occasionally", "frequent", "frequently", "heavy", "light", "sparse",
    "of", "some", "a", "an", "the", "plus", "also", "including", "include",
    "is", "are", "in", "for", "to", "rare", "rarely", "often", "sometimes",
];

/// Remove claims that the author uses long dashes from one prose string.
///
/// Returns `None` — meaning "leave it exactly as it was" — whenever the result
/// would be degenerate or the dash claim survives. A voice profile is LLM prose
/// with no fixed grammar; declining is always safe, and a bad rewrite here
/// would be silently fed into every future draft.
pub fn scrub_dash_claims(s: &str) -> Option<String> {
    if !dash_word_re().is_match(s) {
        return None;
    }

    let ends_with_period = s.trim_end().ends_with('.');
    let starts_upper = s
        .trim_start()
        .chars()
        .next()
        .map_or(false, |c| c.is_uppercase());

    let mut kept: Vec<String> = Vec::new();
    for segment in s.split(',') {
        if !dash_word_re().is_match(segment) {
            let t = segment.trim();
            if !t.is_empty() {
                kept.push(t.to_string());
            }
            continue;
        }

        let mut seg = segment.to_string();
        for re in removal_res() {
            if !dash_word_re().is_match(&seg) {
                break;
            }
            seg = re.replace_all(&seg, " ").into_owned();
        }

        let seg = seg.split_whitespace().collect::<Vec<_>>().join(" ");
        let seg = seg.trim().trim_end_matches('.').trim().to_string();
        if seg.is_empty() || is_filler_only(&seg) {
            continue;
        }
        kept.push(seg);
    }

    if kept.is_empty() {
        return None;
    }

    let mut out = kept.join(", ");
    // A dropped first segment can leave the sentence opening on a connector.
    for lead in ["and ", "or ", "with "] {
        if out.to_lowercase().starts_with(lead) {
            out = out[lead.len()..].to_string();
            break;
        }
    }
    if starts_upper {
        let mut c = out.chars();
        if let Some(first) = c.next() {
            out = first.to_uppercase().collect::<String>() + c.as_str();
        }
    }
    if ends_with_period && !out.ends_with('.') {
        out.push('.');
    }

    // Fail safe: refuse anything that did not actually remove the claim, or
    // that collapsed into a fragment.
    if dash_word_re().is_match(&out) || out.trim().len() < 8 || out == s {
        return None;
    }
    Some(out)
}

fn is_filler_only(seg: &str) -> bool {
    let words: Vec<String> = seg
        .split_whitespace()
        .map(|w| {
            w.trim_matches(|c: char| !c.is_alphanumeric())
                .to_lowercase()
        })
        .filter(|w| !w.is_empty())
        .collect();
    !words.is_empty() && words.iter().all(|w| FILLER.contains(&w.as_str()))
}

/// Apply `scrub_dash_claims` to every string in a stored voice-profile JSON
/// object. Returns `None` when nothing changed.
///
/// Deliberately NOT applied to `voice_examples`: those are verbatim excerpts of
/// mail the user actually sent. They are evidence, and rewriting the record of
/// what someone wrote is a different thing from governing what gets written
/// next.
pub fn scrub_profile_json(json: &str) -> Option<String> {
    let mut value: serde_json::Value = serde_json::from_str(json).ok()?;
    if !scrub_value(&mut value) {
        return None;
    }
    serde_json::to_string(&value).ok()
}

/// In-place scrub of every string in a JSON tree. Returns whether anything
/// changed. Visits every node — no short-circuiting.
pub fn scrub_value(value: &mut serde_json::Value) -> bool {
    match value {
        serde_json::Value::String(s) => match scrub_dash_claims(s) {
            Some(fixed) => {
                *s = fixed;
                true
            }
            None => false,
        },
        serde_json::Value::Array(items) => {
            let mut changed = false;
            for item in items {
                changed |= scrub_value(item);
            }
            changed
        }
        serde_json::Value::Object(map) => {
            let mut changed = false;
            for (_, v) in map.iter_mut() {
                changed |= scrub_value(v);
            }
            changed
        }
        _ => false,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn fix(s: &str) -> String {
        normalize_dashes(s).text
    }

    // ── replacement policy ────────────────────────────────────────────────

    #[test]
    fn a_spaced_em_dash_becomes_a_comma() {
        assert_eq!(fix("Hello — world"), "Hello, world");
    }

    #[test]
    fn an_unspaced_em_dash_becomes_a_comma() {
        // The exact shape the Northwind draft came back with from the voice pass.
        assert_eq!(
            fix("parsed without failures—and identifies the two decisions"),
            "parsed without failures, and identifies the two decisions"
        );
    }

    #[test]
    fn it_never_emits_a_hyphen() {
        // The rule bans the hyphen stand-in explicitly; it is also the
        // substitution every naive de-AI script reaches for.
        let out = fix("Hello — world—and again ― once more");
        assert!(!out.contains('-'), "hyphen leaked into {out:?}");
    }

    #[test]
    fn a_dash_opening_a_line_is_dropped_not_commaed() {
        assert_eq!(fix("— Chris"), "Chris");
        assert_eq!(fix("Thanks,\n— Chris"), "Thanks,\nChris");
    }

    #[test]
    fn a_dash_after_a_closed_tag_opens_a_line() {
        assert_eq!(fix("<p>— Chris</p>"), "<p>Chris</p>");
        assert_eq!(fix("<br/>— note"), "<br/>note");
    }

    #[test]
    fn a_dash_ending_a_line_or_meeting_a_tag_is_dropped() {
        assert_eq!(fix("all done —\nnext line"), "all done\nnext line");
        assert_eq!(fix("<p>all done —</p>"), "<p>all done</p>");
    }

    #[test]
    fn a_numeric_range_becomes_to() {
        assert_eq!(fix("2010—2020"), "2010 to 2020");
        assert_eq!(fix("$3,500 — $7,200"), "$3,500 to $7,200");
    }

    #[test]
    fn a_dash_after_punctuation_collapses_to_one_space() {
        assert_eq!(fix("Hi, — how are you"), "Hi, how are you");
        assert_eq!(fix("Done. — Next"), "Done. Next");
    }

    #[test]
    fn a_run_of_dashes_is_one_replacement() {
        let r = normalize_dashes("a ——— b");
        assert_eq!(r.text, "a, b");
        assert_eq!(r.replaced, 1);

        let r = normalize_dashes("a — — b");
        assert_eq!(r.text, "a, b");
        assert_eq!(r.replaced, 1);
    }

    #[test]
    fn a_separator_rule_of_dashes_disappears() {
        assert_eq!(fix("above\n————————\nbelow"), "above\n\nbelow");
    }

    // ── what it must not touch ────────────────────────────────────────────

    #[test]
    fn tag_interiors_survive_byte_for_byte() {
        let html = r#"<a href="https://x.com/a—b" style="font-family:A—B">text — here</a>"#;
        assert_eq!(
            fix(html),
            r#"<a href="https://x.com/a—b" style="font-family:A—B">text, here</a>"#
        );
    }

    #[test]
    fn style_and_script_bodies_survive() {
        let html = "<style>.a{content:'—'}</style><p>x — y</p><script>var s='—';</script>";
        assert_eq!(
            fix(html),
            "<style>.a{content:'—'}</style><p>x, y</p><script>var s='—';</script>"
        );
    }

    #[test]
    fn a_dash_inside_a_bare_url_is_left_alone() {
        let r = normalize_dashes("see https://x.com/a—b now");
        assert_eq!(r.text, "see https://x.com/a—b now");
        assert_eq!(r.replaced, 0);
    }

    #[test]
    fn an_unspaced_en_dash_is_a_range_and_stays() {
        let r = normalize_dashes("the 2010–2020 window");
        assert_eq!(r.replaced, 0);
        assert_eq!(r.text, "the 2010–2020 window");
    }

    #[test]
    fn a_spaced_en_dash_is_punctuation_and_goes() {
        assert_eq!(fix("Hello – world"), "Hello, world");
    }

    // ── entity spellings ──────────────────────────────────────────────────

    #[test]
    fn html_entity_spellings_are_rewritten_too() {
        // A literal-char replace misses every one of these, and an agent
        // writing `layout: "rich"` HTML produces them routinely.
        assert_eq!(fix("a&mdash;b"), "a, b");
        assert_eq!(fix("a&#8212;b"), "a, b");
        assert_eq!(fix("a&#x2014;b"), "a, b");
        assert_eq!(fix("a&#X2014;b"), "a, b");
        assert_eq!(fix("a &ndash; b"), "a, b");
    }

    #[test]
    fn an_entity_inside_an_attribute_is_left_alone() {
        assert_eq!(
            fix(r#"<span title="a&mdash;b">c&mdash;d</span>"#),
            r#"<span title="a&mdash;b">c, d</span>"#
        );
    }

    // ── safety properties ─────────────────────────────────────────────────

    #[test]
    fn it_is_idempotent() {
        let once = fix("a — b—c ― d &mdash; e");
        let twice = fix(&once);
        assert_eq!(once, twice);
        assert_eq!(normalize_dashes(&once).replaced, 0);
    }

    #[test]
    fn clean_text_is_returned_untouched_and_reports_zero() {
        let input = "Ordinary prose, with commas: and colons (and parentheses).";
        let r = normalize_dashes(input);
        assert_eq!(r.text, input);
        assert_eq!(r.replaced, 0);
        assert!(r.samples.is_empty());
        assert!(!r.changed());
    }

    #[test]
    fn multibyte_text_does_not_panic_or_corrupt() {
        // gotcha #21b: every offset here is a char index, never a byte slice.
        assert_eq!(fix("café — naïve — 日本語"), "café, naïve, 日本語");
        assert_eq!(fix("🙂—🙃"), "🙂, 🙃");
    }

    #[test]
    fn samples_are_capped_and_replacements_are_all_counted() {
        let r = normalize_dashes("a—b c—d e—f g—h i—j");
        assert_eq!(r.replaced, 5);
        assert_eq!(r.samples.len(), MAX_SAMPLES);
        assert!(
            r.samples[0].contains("a, b"),
            "sample should show the fixed text: {:?}",
            r.samples
        );
    }

    #[test]
    fn contains_banned_dash_agrees_with_the_normalizer() {
        assert!(contains_banned_dash("a — b"));
        assert!(contains_banned_dash("a&mdash;b"));
        assert!(!contains_banned_dash("the 2010–2020 window"));
        assert!(!contains_banned_dash("plain prose"));
    }

    // ── voice-profile prose ───────────────────────────────────────────────
    //
    // All four inputs below are the REAL stored strings from the live DB at
    // the time this shipped: two account profiles and two recipient profiles,
    // every one of them prescribing the punctuation the pinned rule bans.

    #[test]
    fn scrubs_the_live_troy_recipient_profile() {
        assert_eq!(
            scrub_dash_claims("Uses em-dashes, parentheses, bullets, and occasional numbered lists.")
                .unwrap(),
            "Parentheses, bullets, and occasional numbered lists."
        );
    }

    #[test]
    fn scrubs_the_live_eric_recipient_profile() {
        assert_eq!(
            scrub_dash_claims(
                "Sparse punctuation overall, with frequent commas and occasional em dashes or \
                 parenthetical asides for clarification."
            )
            .unwrap(),
            "Sparse punctuation overall, with frequent commas and occasional parenthetical asides \
             for clarification."
        );
    }

    #[test]
    fn scrubs_the_live_account_profiles() {
        assert_eq!(
            scrub_dash_claims(
                "Uses commas heavily, occasional em dashes, and frequent colon-led lists."
            )
            .unwrap(),
            "Uses commas heavily, and frequent colon-led lists."
        );
        assert_eq!(
            scrub_dash_claims(
                "Heavy use of colons, bullets, numbered lists, parentheses, and em-dashes/line \
                 breaks for structure."
            )
            .unwrap(),
            "Heavy use of colons, bullets, numbered lists, parentheses, and line breaks for structure."
        );
    }

    #[test]
    fn scrub_declines_rather_than_producing_a_fragment() {
        // Nothing usable would survive — leaving the string alone is safe;
        // a mangled profile is fed into every future draft.
        assert_eq!(scrub_dash_claims("Em-dashes."), None);
        assert_eq!(scrub_dash_claims("em dashes"), None);
    }

    #[test]
    fn scrub_leaves_prose_without_a_dash_claim_alone() {
        assert_eq!(scrub_dash_claims("Short sentences, plain words."), None);
        // The character itself is not a claim about habit — this function
        // governs prose ABOUT dashes; `normalize_dashes` governs the dashes.
        assert_eq!(scrub_dash_claims("Writes tightly — and briefly."), None);
    }

    #[test]
    fn scrub_profile_json_rewrites_only_the_strings_that_claim_it() {
        let json = r#"{"tone":"warm","punctuation_style":"Uses em-dashes, parentheses, bullets, and occasional numbered lists.","typical_length_words":80}"#;
        let out = scrub_profile_json(json).unwrap();
        let v: serde_json::Value = serde_json::from_str(&out).unwrap();
        assert_eq!(v["tone"], "warm");
        assert_eq!(v["typical_length_words"], 80);
        assert_eq!(
            v["punctuation_style"],
            "Parentheses, bullets, and occasional numbered lists."
        );
    }

    #[test]
    fn scrub_profile_json_is_none_when_nothing_claims_it() {
        assert_eq!(
            scrub_profile_json(r#"{"tone":"warm","punctuation_style":"commas and colons"}"#),
            None
        );
    }

    #[test]
    fn scrub_profile_json_declines_invalid_json_instead_of_panicking() {
        assert_eq!(scrub_profile_json("not json"), None);
    }
}
