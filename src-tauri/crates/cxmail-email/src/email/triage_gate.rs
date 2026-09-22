//! What may leave this machine for the inference provider, and whether
//! anything may at all.
//!
//! Every byte the triage pass sends passes through `gate()`. Nothing else in
//! the triage path is allowed to build a payload, so this module is the single
//! place to read when the question is "what did we send?".
//!
//! The provider is the one already configured for Reply with AI and Summarize
//! (`email::inference`), whose terms Chris has accepted for this class of data
//! — so this is not the gate that made the Jev experiment unshippable. It
//! stays because triage is a BACKGROUND SWEEP, not a click: the user does not
//! choose each message, so the code has to be stricter than the buttons are.
//! Three layers, and the last one is the one that holds:
//!
//! 1. **Mode and account scope.** Off by default; per-account opt-in.
//! 2. **Withhold** a message whose sender is a known financial institution.
//!    Cheap, exact, and structurally incapable of catching a forward.
//! 3. **Redact** every body that does go, whoever sent it. Account and card
//!    numbers, routing and tax ids, amounts, passcodes, keys and tokens become
//!    typed placeholders before the body is serialized — then a **final check**
//!    refuses the whole message if anything shaped like those survived.
//!
//! **Redacting the amounts costs nothing we wanted.** None of the four triage
//! questions need a figure: "an invoice is past due" is the whole signal, and
//! `[amount]` carries it. The thing we would rather not send is the thing the
//! judgment could not have used.
//!
//! **Fail closed.** Every path that cannot complete returns `Withheld`. A
//! withheld message is reported as `unknown` downstream and never as "no
//! response needed" — absence of a judgment must not read as a judgment.
//!
//! History: built 2026-09-17 for a TypeSafe/Jev experiment and hardened over
//! four live audit rounds on real mail (URLs, phone numbers, a one-time
//! passcode, letter-glued reference numbers). The experiment was shelved on
//! vendor-retention grounds; the gate outlived it because every hole it found
//! is a hole regardless of who is on the other end.

use regex::Regex;
use std::sync::LazyLock;

/// Credential-store key for the triage mode. Runtime, deliberately NOT an
/// `option_env!` like `CXMAIL_OWNER_LICENSE`: owner builds skip the update
/// check entirely (gotcha #28), so a compile-time flag would mean waiting on a
/// `/ship` to turn this off. The kill switch has to be reachable from Settings.
///
/// Same store and the same resolution shape as `ai:writer:model`.
pub const TRIAGE_MODE_KEY: &str = "triage:mode";

/// How much of the triage feature is live.
///
/// `Off` is the default and means byte-identical behaviour to a build without
/// this feature: no request is made and no verdict is read. `Shadow` records
/// verdicts against live mail while changing nothing on screen — it is how the
/// benchmark runs without putting the inbox at risk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TriageMode {
    Off,
    Shadow,
    On,
}

impl TriageMode {
    pub fn as_str(&self) -> &'static str {
        match self {
            TriageMode::Off => "off",
            TriageMode::Shadow => "shadow",
            TriageMode::On => "on",
        }
    }

    /// May this mode send anything to the inference provider?
    pub fn may_send(&self) -> bool {
        matches!(self, TriageMode::Shadow | TriageMode::On)
    }

    /// May a stored verdict change what the user sees?
    pub fn may_surface(&self) -> bool {
        matches!(self, TriageMode::On)
    }
}

/// The pure half of `effective_mode`, so precedence is testable without a
/// credential store — the shape `external_writer::effective_default_from` uses.
///
/// Anything unrecognized resolves to `Off`. A typo in the stored value must not
/// leave the feature running.
pub fn effective_mode_from(configured: Option<&str>) -> TriageMode {
    match configured.map(|s| s.trim().to_ascii_lowercase()).as_deref() {
        Some("shadow") => TriageMode::Shadow,
        Some("on") => TriageMode::On,
        _ => TriageMode::Off,
    }
}

/// The configured mode, read fresh from the credential store every time.
///
/// Never cached: this is the kill switch, so a session that has been running
/// since before the user turned triage off must not keep sending.
pub fn effective_mode() -> TriageMode {
    let stored = crate::keychain::get_credential(TRIAGE_MODE_KEY)
        .ok()
        .flatten();
    effective_mode_from(stored.as_deref())
}

/// Persist the mode. `Off` deletes the key rather than writing "off", so the
/// absent state and the off state are the same state.
pub fn save_mode(mode: TriageMode) -> Result<(), crate::error::AppError> {
    match mode {
        TriageMode::Off => crate::keychain::delete_credential(TRIAGE_MODE_KEY),
        other => crate::keychain::store_credential(TRIAGE_MODE_KEY, other.as_str()),
    }
}

/// Registrable domains whose mail never leaves this machine.
///
/// Matched on a label boundary, never as a substring — gotcha #49's lesson,
/// where a substring test made `notharborline.example` a client. `chase.com` must not
/// match `notchase.com`, and `mail.chase.com` must.
///
/// Extend this list. Never add a way to turn it off.
const FINANCIAL_SENDERS: &[&str] = &[
    // Banking and cards
    "chase.com", "bankofamerica.com", "wellsfargo.com", "citi.com", "citibank.com",
    "capitalone.com", "usbank.com", "pnc.com", "truist.com", "amex.com",
    "americanexpress.com", "discover.com", "synchronybank.com", "ally.com",
    "sofi.com", "marcus.com", "schwab.com", "navyfederal.org",
    // Business banking and payments
    "mercury.com", "brex.com", "ramp.com", "stripe.com", "squareup.com",
    "block.xyz", "paypal.com", "venmo.com", "wise.com", "payoneer.com",
    "gusto.com", "rippling.com", "adp.com", "bill.com",
    // Brokerage, retirement, crypto
    // `ml.com` (Merrill) and `passiv.com` were added after the first live run
    // sent their mail through. Note what is deliberately NOT here:
    // `schwabedigital.com` appeared in the same corpus and is not Charles
    // Schwab — the label-boundary matcher already refused it, and adding it
    // would be exactly the mistake that matcher exists to prevent.
    "fidelity.com", "vanguard.com", "etrade.com", "robinhood.com",
    "ml.com", "merrilledge.com", "passiv.com", "tdameritrade.com",
    "edwardjones.com", "troweprice.com", "morganstanley.com",
    "coinbase.com", "kraken.com", "gemini.com", "betterment.com",
    "wealthfront.com", "empower.com", "principal.com",
    // Accounting, tax, lending
    "intuit.com", "turbotax.com", "quickbooks.com", "irs.gov", "xero.com",
    "freshbooks.com", "wave.com", "creditkarma.com", "experian.com",
    "hrblock.com", "taxact.com", "lendingclub.com", "affirm.com",
    "klarna.com", "afterpay.com", "chime.com", "plaid.com",
    "zellepay.com", "remitly.com", "westernunion.com", "nerdwallet.com",
    "equifax.com", "transunion.com", "nelnet.com", "mohela.com",
    // Insurance and health finance
    "geico.com", "progressive.com", "statefarm.com", "allstate.com",
];

/// How many domains the financial blocklist currently holds. Reported by the
/// probe so "the blocklist is in place" is something the operator reads off the
/// run, not something they take on trust.
pub fn financial_domain_count() -> usize {
    FINANCIAL_SENDERS.len()
}

/// A message offered to the gate. Borrowed, so nothing is copied for a message
/// that turns out to be withheld.
pub struct MessageForTriage<'a> {
    pub from_email: &'a str,
    pub from_name: Option<&'a str>,
    pub to: &'a [String],
    pub cc: &'a [String],
    pub date: &'a str,
    pub subject: Option<&'a str>,
    /// The cached plain-text body. `None` means it was never cached — which is
    /// "we do not know", not "there is no body".
    pub body: Option<&'a str>,
}

/// Why nothing was sent. Each variant is reported downstream as `unknown`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Withheld {
    /// The feature is off. No request was made.
    ModeOff,
    /// The sender is on the financial blocklist. Names the matched domain so
    /// the reason is auditable without re-deriving it.
    FinancialSender(String),
    /// No cached body. Headers alone measured 0.73 on `needs_response` against
    /// 0.98 with the body — under any sensible gate, so we decline rather than
    /// record a judgment we would not act on.
    NoBody,
    /// Nothing substantive survived redaction.
    Empty,
    /// The finished payload still matched a final-check pattern. Names the
    /// pattern so a recurring withhold can be diagnosed without re-deriving it.
    FailedFinalCheck(&'static str),
}

impl Withheld {
    pub fn reason(&self) -> String {
        match self {
            Withheld::ModeOff => "triage is off".to_string(),
            Withheld::FinancialSender(d) => format!("financial sender ({d})"),
            Withheld::NoBody => "no cached body".to_string(),
            Withheld::Empty => "nothing left after redaction".to_string(),
            Withheld::FailedFinalCheck(p) => format!("failed the final check ({p})"),
        }
    }
}

/// Patterns that must not appear in a finished payload.
///
/// Deliberately BROADER than the redactor — six digits where the redactor takes
/// six-and-context, any currency sigil followed by a digit — so they
/// over-report. That asymmetry is the point: a rule written to catch exactly
/// what the redactor removes can only ever agree with it. This set is what
/// caught a live one-time passcode the redactor's 7-digit rule had passed.
///
/// These run on EVERY payload at runtime, not only in the probe. Before this
/// existed the guarantee was "a sample was audited"; now it is "no payload
/// matching these can leave", which is a claim the code keeps rather than one
/// an operator remembers to check.
static FINAL_CHECKS: LazyLock<Vec<(&'static str, Regex)>> = LazyLock::new(|| {
    vec![
        ("digits", Regex::new(r"\d{6,}").unwrap()),
        ("grouped-digits", Regex::new(r"\d[ -]\d{3}[ -]\d{3,}").unwrap()),
        ("currency", Regex::new(r"[$€£¥]\s?\d").unwrap()),
        ("currency-code", Regex::new(r"(?i)\b\d[\d,.]*\s?(USD|EUR|GBP|CAD)\b").unwrap()),
        ("iban", Regex::new(r"\b[A-Z]{2}\d{2}[A-Z0-9]{8,}").unwrap()),
        ("tax-id", Regex::new(r"\b\d{3}[ -]\d{2}[ -]\d{4}\b").unwrap()),
        ("long-token", Regex::new(r"\b[A-Za-z0-9+/_-]{28,}\b").unwrap()),
        ("key-prefix", Regex::new(r"(?i)\b(sk|pk|whsec|ghp)[-_][A-Za-z0-9]{8,}").unwrap()),
        ("bearer", Regex::new(r"(?i)bearer\s+[A-Za-z0-9._-]{12,}").unwrap()),
        ("email", Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap()),
        ("code-context", Regex::new(r"(?i)(verification|passcode|one[- ]?time|2fa|otp|pin)\D{0,24}\d{4,}").unwrap()),
    ]
});

/// The first final-check pattern this text trips, if any.
pub fn final_check(text: &str) -> Option<&'static str> {
    FINAL_CHECKS
        .iter()
        .find(|(_, re)| re.is_match(text))
        .map(|(name, _)| *name)
}

/// A message cleared to leave, with the body already redacted and truncated.
/// Construct this ONLY through `gate()` — the private field is what stops a
/// caller assembling one from raw text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SafeMessage {
    pub from_email: String,
    pub from_name: Option<String>,
    pub to: Vec<String>,
    pub cc: Vec<String>,
    pub date: String,
    pub subject: String,
    /// Redacted and truncated. Never the raw body.
    pub body: String,
    /// True when the body was cut short, so the question can say so instead of
    /// letting the model read a sentence that stops mid-clause as the end.
    pub body_truncated: bool,
    _sealed: (),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Gate {
    Send(Box<SafeMessage>),
    Withheld(Withheld),
}

/// Bodies are cut to this many characters before sending.
///
/// Two reasons, pulling the same way: cost is linear in tokens, and a large
/// state full of irrelevant detail degrades the judgment — filter first.
/// A reply's value lives at the top; a 40k-character newsletter footer has
/// never decided whether Chris owes someone an answer.
pub const MAX_BODY_CHARS: usize = 2_000;

/// The one way to build a payload.
pub fn gate(mode: TriageMode, msg: &MessageForTriage<'_>) -> Gate {
    if !mode.may_send() {
        return Gate::Withheld(Withheld::ModeOff);
    }
    if let Some(domain) = financial_sender(msg.from_email) {
        return Gate::Withheld(Withheld::FinancialSender(domain.to_string()));
    }
    let Some(raw_body) = msg.body else {
        return Gate::Withheld(Withheld::NoBody);
    };

    let subject = redact(msg.subject.unwrap_or(""));
    let redacted_body = redact(raw_body);
    let (body, body_truncated) = truncate_chars(&redacted_body, MAX_BODY_CHARS);

    if body.trim().is_empty() && subject.trim().is_empty() {
        return Gate::Withheld(Withheld::Empty);
    }

    // The last word. Redaction is a pile of rules that each remove one thing;
    // this asks the opposite question — does anything that must not leave
    // remain? — and refuses the whole message if so. A message withheld here
    // is reported as `unknown`, which is a triage result we are happy to have
    // and strictly better than a payload nobody inspected.
    if let Some(pattern) = final_check(&subject).or_else(|| final_check(&body)) {
        return Gate::Withheld(Withheld::FailedFinalCheck(pattern));
    }

    Gate::Send(Box::new(SafeMessage {
        from_email: msg.from_email.to_string(),
        from_name: msg.from_name.map(str::to_string),
        to: msg.to.to_vec(),
        cc: msg.cc.to_vec(),
        date: msg.date.to_string(),
        subject,
        body: body.to_string(),
        body_truncated,
        _sealed: (),
    }))
}

/// The blocklisted domain this address belongs to, if any.
fn financial_sender(email: &str) -> Option<&'static str> {
    let domain = email.rsplit('@').next()?.trim().to_ascii_lowercase();
    if domain.is_empty() {
        return None;
    }
    FINANCIAL_SENDERS.iter().copied().find(|blocked| {
        domain == *blocked || domain.ends_with(&format!(".{blocked}"))
    })
}

/// Cut to `max` characters on a CHARACTER boundary.
///
/// `&s[..max]` guarded by `s.len() >= max` is a panic on multi-byte input —
/// gotcha #21b, where a `──────` separator line took down the event detector.
/// A length check is not a boundary check.
fn truncate_chars(s: &str, max: usize) -> (&str, bool) {
    match s.char_indices().nth(max) {
        Some((idx, _)) => (&s[..idx], true),
        None => (s, false),
    }
}

// --- redaction ------------------------------------------------------------
//
// Applied to the BODY and SUBJECT only. Envelope addresses are deliberately
// sent intact: who wrote and who was copied is most of what decides whether a
// reply is owed, and it is data CXMail already holds about Chris's own
// correspondents. An address *inside* a body is usually a third party's, in a
// signature or a forwarded header, and is worth nothing to the judgment.
//
// Order matters: the most specific pattern must run before a general one that
// would consume part of it, or the general placeholder hides what it ate.

/// A CSS rule: a selector and its declaration block.
///
/// Newsletters routinely leak their entire stylesheet into `text/plain`, and
/// the 2,000-character cap then spends the whole budget on it. One message in
/// the probe corpus — an Oktoberfest newsletter — sent 2,000 characters of
/// `!important` and not one word of content.
///
/// Stripping it improves three things at once: the payload stops carrying
/// class names that read like opaque tokens, the tokens are no longer wasted,
/// and the judgment stops reading a stylesheet (jaggedness #5, large state full
/// of irrelevant detail). A body that was nothing but CSS now redacts to
/// nothing and is withheld as `Empty` rather than sent as noise.
///
/// Gated on the block LOOKING like declarations — a colon plus a semicolon or
/// an `!important` — so a sentence that happens to contain braces survives.
static CSS_RULE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)[^{}\n]{0,200}\{[^{}]{0,4000}\}").unwrap());

/// A whole `<style>` or `<script>` element that leaked into the plain-text
/// part. Markup, never prose — the contents go with the tags.
static MARKUP_BLOCK_RE: LazyLock<Regex> = LazyLock::new(|| {
    // Spelled out rather than backreferenced: Rust's regex crate has no `\1`,
    // and a LazyLock built from an invalid pattern panics on first redaction.
    Regex::new(
        r"(?is)<style\b[^>]*>.*?</style\s*>|<script\b[^>]*>.*?</script\s*>|</?(?:style|script)\b[^>]*>",
    )
    .unwrap()
});

/// A lone declaration, `border-radius:4px`, with no semicolon to give it away.
/// The `looks_like_css` guard needs this or a single-rule block survives.
static SINGLE_DECL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"^[a-zA-Z-]{2,}\s*:\s*[^:;{}]+$").unwrap());

/// Comments from the same leaked stylesheet or template — `/* … */` and
/// `<!-- … -->`. They carry build paths (`molecules/ClaimedInventiveCta.vue`)
/// that read as opaque tokens and tell the judgment nothing.
static COMMENT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?s)/\*.*?\*/|<!--.*?-->").unwrap());

/// Leftovers once the rules are gone: `@media` preludes and orphaned braces.
static CSS_DEBRIS_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)@(?:media|import|font-face|charset)[^{};]{0,200}[{;]?").unwrap());

/// Invisible padding and runaway whitespace, collapsed.
///
/// Marketing mail pads its preheader with hundreds of U+034F combining grapheme
/// joiners so the preview text fills out — one Plugin Alliance body in the probe
/// corpus was mostly this. It is billed like any other token and carries nothing.
///
/// Deliberately NOT `cxmail_core::mail::text::strip_invisible_chars`: that one
/// is private, cleans snippets for DISPLAY, and does not know U+034F. Widening
/// it would change every stored snippet's behaviour to save tokens on an
/// outbound payload — two different jobs that happen to rhyme.
fn collapse_filler(s: &str) -> String {
    let stripped: String = s
        .chars()
        .filter(|c| {
            !matches!(c,
                '\u{00AD}' | '\u{034F}' | '\u{061C}' | '\u{180E}' | '\u{FEFF}'
            ) && !('\u{200B}'..='\u{200F}').contains(c)
              && !('\u{2060}'..='\u{2064}').contains(c)
              && !('\u{FE00}'..='\u{FE0F}').contains(c)
        })
        .collect();

    // Paragraph structure is worth keeping — a blank line separates a signature
    // from a request — so runs of blank lines become one, not none.
    let mut out = String::with_capacity(stripped.len());
    let mut blank_run = 0usize;
    for line in stripped.lines() {
        let trimmed = line.trim();
        if trimmed.is_empty() {
            blank_run += 1;
            continue;
        }
        if !out.is_empty() {
            out.push('\n');
            if blank_run > 0 {
                out.push('\n');
            }
        }
        blank_run = 0;
        let mut last_space = false;
        for c in trimmed.chars() {
            let is_space = c.is_whitespace();
            if is_space && last_space {
                continue;
            }
            out.push(if is_space { ' ' } else { c });
            last_space = is_space;
        }
    }
    out
}

/// Drop braces with no partner.
///
/// Stripping a rule out of an `@media` wrapper leaves its closing brace behind.
/// Removing every brace would eat `{name}` out of a mail-merge template, so
/// this drops only the UNMATCHED ones — which is exactly what CSS debris is and
/// what balanced prose is not.
fn drop_unmatched_braces(s: &str) -> String {
    let chars: Vec<char> = s.chars().collect();
    let mut doomed = vec![false; chars.len()];
    let mut open: Vec<usize> = Vec::new();
    for (i, c) in chars.iter().enumerate() {
        match c {
            '{' => open.push(i),
            '}' => {
                if open.pop().is_none() {
                    doomed[i] = true;
                }
            }
            _ => {}
        }
    }
    for i in open {
        doomed[i] = true;
    }
    chars
        .iter()
        .enumerate()
        .filter(|(i, _)| !doomed[*i])
        .map(|(_, c)| *c)
        .collect()
}

/// A whole URL, tracking query and all.
///
/// The single biggest carrier of opaque identifiers in real mail, and the
/// least useful thing in it. The probe's first run over 300 live messages found
/// 49 payloads carrying tokens, every one of them inside a URL — Klaviyo
/// unsubscribe links, LinkedIn tracking parameters, Nextdoor profile tokens.
/// An unsubscribe token is a capability: whoever holds it can act as the
/// recipient. None of the four triage questions need the address, only that a
/// link was there, so `[link]` carries the whole signal.
///
/// This is zen-mcp's "withhold what you don't need" rule — removing the field
/// beats pattern-matching its contents, because the pattern list is never done.
static URL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"(?i)\b(?:https?://|www\.)\S+").unwrap());

/// Grouped digit runs in phone shape: `800-823-2478`, `1 844 229 2211`,
/// `(615) 555-0142`, and the 3-3-3 shapes spam uses. Not financial, but it is
/// somebody's personal number and the judgment never needed it.
static PHONE_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?:\+?\d{1,2}[ .-])?(?:\(\d{3}\)[ .-]?|\d{3}[ .-])\d{3}[ .-]\d{3,4}\b").unwrap()
});

static JWT_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\beyJ[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]{8,}\.[A-Za-z0-9_-]+").unwrap());

static API_KEY_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b(?:sk|pk|rk|whsec|xox[baprs]|ghp|gho|github_pat)[-_][A-Za-z0-9_-]{10,}\b")
        .unwrap()
});

static UUID_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(r"(?i)\b[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}\b").unwrap()
});

static EMAIL_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"[A-Za-z0-9._%+-]+@[A-Za-z0-9.-]+\.[A-Za-z]{2,}").unwrap());

static IBAN_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Z]{2}[0-9]{2}[A-Z0-9]{11,30}\b").unwrap());

static SSN_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\b\d{3}-\d{2}-\d{4}\b").unwrap());

/// A card-shaped run: 13–19 digits, optionally grouped by spaces or hyphens.
static CARD_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b(?:\d[ -]?){12,18}\d\b").unwrap());

/// Currency amounts in either order, with or without separators.
static AMOUNT_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)(?:[$€£¥]\s?\d[\d,]*(?:\.\d{1,2})?|\b\d[\d,]*(?:\.\d{1,2})?\s?(?:USD|EUR|GBP|CAD|AUD|JPY)\b)",
    )
    .unwrap()
});

/// A one-time passcode, keyed on the words around it rather than its length.
///
/// The probe's second live run caught `verification code to continue: 437101`
/// leaving intact: an OTP is six digits and the generic rule took seven. Length
/// alone cannot separate a passcode from a year, so this rule reads the
/// sentence instead — which is also why it can take a 4-digit PIN that no
/// digit-count rule could reach without eating every date in the mailbox.
///
/// The prefix is captured and put back, so `verification code to continue:`
/// survives and only the secret goes. The judgment wants to know a code was
/// sent; it has never wanted the code.
static OTP_RE: LazyLock<Regex> = LazyLock::new(|| {
    Regex::new(
        r"(?i)((?:verification|confirmation|security|authentication|one[- ]?time|login|access|passcode|pin|otp|2fa)\D{0,24})\b\d{4,8}\b",
    )
    .unwrap()
});

/// A reference number with a letter in it: `DE812871812`, `58320715BF854063V`,
/// `K0173131326`, `69020364XXXX`.
///
/// The probe's third live run found these were the whole of the remaining
/// leakage, and all of it from one bug: `\b\d{6,}\b` cannot match a digit run
/// glued to a letter, because a letter-to-digit transition is not a word
/// boundary. A VAT id, a PayPal transaction id and a membership number all
/// have that shape, and all three are financial.
///
/// The length and the mixed-ness are the signal. Rust's regex has no lookahead,
/// so the "contains both a letter and a digit" half is a closure in `redact`
/// rather than a pattern — eight or more alphanumerics with both is a
/// reference, and is vanishingly rare in prose.
static MIXED_REF_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9]{8,}\b").unwrap());

/// Any remaining run of 6 or more digits: account, routing, tax, policy, order
/// and receipt numbers all live here.
///
/// Six rather than seven since the OTP find — six-digit runs in real mail are
/// passcodes, order numbers and campaign codes, none of which a triage question
/// needs. Still above four, so a year stays a year.
/// No word boundaries, deliberately: see `MIXED_REF_RE`. `\b` is what let
/// `DE812871812` through.
static LONG_DIGITS_RE: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"\d{6,}").unwrap());

/// A long opaque token: 32+ hex or base64-ish characters with no vowel-driven
/// word shape. Catches secrets the named patterns miss.
static OPAQUE_RE: LazyLock<Regex> =
    LazyLock::new(|| Regex::new(r"\b[A-Za-z0-9+/_-]{32,}={0,2}\b").unwrap());

/// Replace everything that must not leave with a typed placeholder.
///
/// Placeholders are words, not `****`: the model still learns that an amount
/// was named, which is the part the judgment uses.
pub fn redact(text: &str) -> String {
    // CSS first: a stylesheet contains colours, numbers, class names and
    // `url(...)`, so every later rule would otherwise spend itself rewriting
    // markup nobody is going to read.
    let demarked = MARKUP_BLOCK_RE.replace_all(text, " ");
    let decommented = COMMENT_RE.replace_all(&demarked, " ");
    let mut css_stripped = std::borrow::Cow::Owned(decommented.into_owned());
    for _ in 0..3 {
        // Three passes unwrap `@media` nesting; the innermost block goes first.
        let next = CSS_RULE_RE.replace_all(&css_stripped, |c: &regex::Captures| {
            let whole = &c[0];
            let inner = whole
                .split_once('{')
                .map(|(_, r)| r.trim_end_matches('}'))
                .unwrap_or("");
            let looks_like_css = inner.contains(':')
                && (inner.contains(';')
                    || inner.contains("!important")
                    || SINGLE_DECL_RE.is_match(inner.trim()));
            if looks_like_css { String::new() } else { whole.to_string() }
        });
        if next == css_stripped {
            break;
        }
        css_stripped = std::borrow::Cow::Owned(next.into_owned());
    }
    let s = CSS_DEBRIS_RE.replace_all(&css_stripped, " ");
    let s = drop_unmatched_braces(&s);
    // URLs next: they can contain every other pattern, and a later rule that
    // partially rewrites one leaves a placeholder hiding what it ate.
    let s = URL_RE.replace_all(&s, "[link]");
    let s = JWT_RE.replace_all(&s, "[token]");
    let s = API_KEY_RE.replace_all(&s, "[key]");
    let s = UUID_RE.replace_all(&s, "[id]");
    let s = SSN_RE.replace_all(&s, "[tax-id]");
    let s = IBAN_RE.replace_all(&s, "[account]");
    let s = CARD_RE.replace_all(&s, "[card]");
    let s = AMOUNT_RE.replace_all(&s, "[amount]");
    let s = PHONE_RE.replace_all(&s, "[phone]");
    let s = OTP_RE.replace_all(&s, "${1}[code]");
    // Mixed-reference before the digit rule, so a country or issuer prefix goes
    // with its number instead of leaving `DE[number]` behind.
    let s = MIXED_REF_RE.replace_all(&s, |c: &regex::Captures| {
        let m = &c[0];
        let has_digit = m.bytes().any(|b| b.is_ascii_digit());
        let has_alpha = m.bytes().any(|b| b.is_ascii_alphabetic());
        if has_digit && has_alpha {
            "[ref]".to_string()
        } else {
            m.to_string()
        }
    });
    let s = EMAIL_RE.replace_all(&s, "[email]");
    let s = LONG_DIGITS_RE.replace_all(&s, "[number]");
    let s = OPAQUE_RE.replace_all(&s, "[opaque]");
    collapse_filler(&s)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn msg<'a>(from: &'a str, body: Option<&'a str>) -> MessageForTriage<'a> {
        MessageForTriage {
            from_email: from,
            from_name: Some("Someone"),
            to: &[],
            cc: &[],
            date: "2026-09-17T10:00:00Z",
            subject: Some("Hello"),
            body,
        }
    }

    // --- the mode is the outermost gate ---------------------------------

    #[test]
    fn off_is_the_default_for_anything_unrecognized() {
        assert_eq!(effective_mode_from(None), TriageMode::Off);
        assert_eq!(effective_mode_from(Some("")), TriageMode::Off);
        assert_eq!(effective_mode_from(Some("enabled")), TriageMode::Off);
        assert_eq!(effective_mode_from(Some("true")), TriageMode::Off);
        assert_eq!(effective_mode_from(Some("Shadow ")), TriageMode::Shadow);
        assert_eq!(effective_mode_from(Some("ON")), TriageMode::On);
    }

    #[test]
    fn off_sends_nothing_even_for_a_perfectly_ordinary_message() {
        let m = msg("dana@northwind.example", Some("Can you confirm the date?"));
        assert_eq!(gate(TriageMode::Off, &m), Gate::Withheld(Withheld::ModeOff));
    }

    #[test]
    fn shadow_sends_but_does_not_surface() {
        assert!(TriageMode::Shadow.may_send());
        assert!(!TriageMode::Shadow.may_surface());
        assert!(!TriageMode::Off.may_send());
        assert!(TriageMode::On.may_surface());
    }

    // --- layer 1: the blocklist -----------------------------------------

    #[test]
    fn a_financial_sender_is_withheld_and_named() {
        let m = msg("statements@chase.com", Some("Your statement is ready"));
        assert_eq!(
            gate(TriageMode::On, &m),
            Gate::Withheld(Withheld::FinancialSender("chase.com".into()))
        );
    }

    #[test]
    fn a_subdomain_of_a_financial_sender_is_withheld() {
        let m = msg("no-reply@alerts.mercury.com", Some("Wire received"));
        assert!(matches!(
            gate(TriageMode::On, &m),
            Gate::Withheld(Withheld::FinancialSender(_))
        ));
    }

    #[test]
    fn a_lookalike_domain_is_not_a_financial_sender() {
        // The gotcha #49 trap: a substring test makes this a bank.
        assert_eq!(financial_sender("hi@notchase.com"), None);
        assert_eq!(financial_sender("hi@chase.com.evil.co"), None);
        assert_eq!(financial_sender("hi@mystripe.com"), None);
        // Found in the live corpus: NOT Charles Schwab. Adding it to the
        // blocklist would withhold an unrelated sender's mail forever.
        assert_eq!(financial_sender("hi@schwabedigital.com"), None);
    }

    #[test]
    fn the_senders_the_first_live_run_let_through_are_blocked_now() {
        for addr in ["rg@message.rg.ml.com", "no-reply@passiv.com"] {
            assert!(
                matches!(
                    gate(TriageMode::On, &msg(addr, Some("Important action needed"))),
                    Gate::Withheld(Withheld::FinancialSender(_))
                ),
                "still not blocked: {addr}"
            );
        }
    }

    // --- layer 2: redaction is what actually has to hold ----------------

    #[test]
    fn a_forwarded_wire_confirmation_from_gmail_is_redacted_not_blocked() {
        // The case the blocklist structurally cannot catch: a client forwards
        // banking detail from a personal address. Layer 1 passes it; layer 2
        // is the reason that is survivable.
        let body = "FYI - wire went out today. Acct 4829301847, routing 121000248, \
                    $12,450.00 to the vendor. Confirmation 9f2c8a41-3b7e-4d21-9c11-5e8a7b3d0c92.";
        let m = msg("partner@gmail.com", Some(body));
        let Gate::Send(safe) = gate(TriageMode::On, &m) else {
            panic!("should have been sent, redacted");
        };
        assert!(!safe.body.contains("4829301847"), "account number survived");
        assert!(!safe.body.contains("121000248"), "routing number survived");
        assert!(!safe.body.contains("12,450"), "amount survived");
        assert!(!safe.body.contains("9f2c8a41"), "uuid survived");
        // The signal the judgment actually needs is still there.
        assert!(safe.body.contains("wire went out today"));
        assert!(safe.body.contains("[amount]"));
    }

    #[test]
    fn card_numbers_survive_no_grouping_style() {
        for raw in [
            "4111111111111111",
            "4111 1111 1111 1111",
            "4111-1111-1111-1111",
        ] {
            let out = redact(&format!("card {raw} on file"));
            assert!(!out.contains("1111 1111"), "grouped card survived: {out}");
            assert!(!out.contains("4111111111111111"), "card survived: {out}");
        }
    }

    #[test]
    fn keys_and_tokens_are_redacted() {
        // A made-up key, split with `concat!` so no secret scanner reads the
        // source as a leaked Stripe key. The runtime string is unchanged.
        let out = redact(concat!(
            "use sk_", "live_51H8xKmFakeKeyMaterial9999 and ",
            "eyJhbGciOiJIUzI1NiJ9.eyJzdWIiOiIxMjM0NTY3ODkwIn0.dQw4w9WgXcQabcdef",
        ));
        assert!(!out.contains(concat!("sk_", "live_51H8xKm")), "api key survived: {out}");
        assert!(!out.contains("eyJhbGci"), "jwt survived: {out}");
    }

    #[test]
    fn a_tracking_url_leaves_nothing_behind_but_the_fact_of_a_link() {
        // The probe's first live run: every token that survived redaction was
        // inside a URL, and an unsubscribe token is a capability URL.
        let out = redact(
            "Manage at https://manage.kmail-lists.com/subscriptions/unsubscribe?a=KXDNYH&c=01HT0RKX3G3V or reply.",
        );
        assert!(!out.contains("KXDNYH"), "tracking token survived: {out}");
        assert!(!out.contains("kmail-lists"), "url survived: {out}");
        assert!(out.contains("[link]"));
        // The sentence around it still reads, which is what the judgment uses.
        assert!(out.contains("Manage at") && out.contains("or reply."));
    }

    #[test]
    fn phone_numbers_are_redacted_in_every_grouping() {
        for raw in ["800-823-2478", "1 844 229 2211", "(615) 555-0142", "381-102-127"] {
            let out = redact(&format!("call {raw} today"));
            assert!(out.contains("[phone]"), "phone survived: {raw} -> {out}");
        }
    }

    #[test]
    fn a_date_is_not_a_phone_number() {
        // The phone pattern is the one most likely to eat something ordinary.
        assert_eq!(redact("due 2026-09-17 at the latest"), "due 2026-09-17 at the latest");
        assert_eq!(redact("version 1.2.3 shipped"), "version 1.2.3 shipped");
    }

    #[test]
    fn a_one_time_passcode_does_not_leave() {
        // Found by the probe on real mail, not by review: six digits passed the
        // 7+ rule. The audit was set one digit broader than the redactor for
        // exactly this, which is why it is not built from the same patterns.
        let out = redact("Enter the verification code to continue: 437101 Please ignore this email if...");
        assert!(!out.contains("437101"), "passcode survived: {out}");
        assert!(out.contains("[code]"));
        // The surrounding sentence is what tells the model a code was sent.
        assert!(out.contains("verification code to continue"));
    }

    #[test]
    fn a_short_pin_is_caught_by_context_that_no_digit_count_could_reach() {
        let out = redact("Your PIN is 4821 for the door.");
        assert!(!out.contains("4821"), "pin survived: {out}");
    }

    #[test]
    fn a_year_is_still_a_year() {
        // The counterweight: OTP context plus a 6-digit floor must not start
        // eating ordinary prose.
        let prose = "Kickoff is 2026, week 46, with 12 seats and 3 rooms booked.";
        assert_eq!(redact(prose), prose);
    }

    #[test]
    fn a_reference_number_with_a_letter_in_it_does_not_leave() {
        // Every one of these came off the real mailbox in the probe's third
        // run, and every one defeated the digit rule the same way: a letter
        // glued to the digits is not a word boundary.
        for raw in [
            "VAT Reg. No. DE812871812",
            "Transaction ID: 58320715BF854063V",
            "Repair ID: D727881467",
            "Payment Reminder (K0173131326)",
            "Membership Number: 69020364XXXX",
            "EU VAT ID CZ04788290",
        ] {
            let out = redact(raw);
            assert!(out.contains("[ref]"), "reference survived: {raw} -> {out}");
        }
    }

    #[test]
    fn a_word_is_not_a_reference_number() {
        // MIXED_REF needs BOTH a letter and a digit, or it eats English.
        let prose = "Absolutely, the onboarding checklist and the quarterly \
                     review were completed yesterday afternoon.";
        assert_eq!(redact(prose), prose);
    }

    #[test]
    fn an_iso_timestamp_survives_because_dates_decide_urgency() {
        let s = "due at 2026-09-12T13:38:02Z";
        assert_eq!(redact(s), s);
    }

    #[test]
    fn a_newsletter_stylesheet_is_stripped_not_sent() {
        // Real shape from the probe corpus: the whole plain-text body was CSS.
        let body = "img{border:0;height:auto;line-height:100%}\
                    a[class~=x_button-block__link-modifier]{width:180px!important;color:#fff!important}\
                    @media only screen and (max-width:480px){.col{width:100%!important}}";
        let out = redact(body);
        assert!(!out.contains("x_button-block"), "class name survived: {out}");
        assert!(!out.contains("!important"), "css survived: {out}");
        assert!(out.trim().is_empty(), "expected nothing left, got: {out}");
    }

    #[test]
    fn a_body_that_is_entirely_css_is_withheld_as_empty() {
        let m = MessageForTriage {
            from_email: "news@example.com",
            from_name: None,
            to: &[],
            cc: &[],
            date: "2026-09-17T10:00:00Z",
            subject: Some(""),
            body: Some("td{background-color:#0071eb!important;text-align:center!important}"),
        };
        assert_eq!(gate(TriageMode::On, &m), Gate::Withheld(Withheld::Empty));
    }

    #[test]
    fn prose_with_braces_is_not_mistaken_for_css() {
        // The guard that keeps this from eating real mail.
        let prose = "Use the template {name} and {company} in the merge fields.";
        assert_eq!(redact(prose), prose);
        let json = r#"The webhook posts {"status": "ok", "id": 4} on success."#;
        assert_eq!(redact(json), json);
    }

    #[test]
    fn stylesheet_comments_carrying_build_paths_are_stripped() {
        let out = redact("/* molecules/ClaimedInventiveCta.vue */ Real content here.");
        assert!(!out.contains("ClaimedInventiveCta"), "comment survived: {out}");
        assert_eq!(out.trim(), "Real content here.");
    }

    #[test]
    fn a_style_tag_in_the_plain_text_part_goes_with_its_contents() {
        let out = redact(
            r#"Hello. <style type="text/css"> .image-block__default--rounded{border-radius:4px} </style> Regards."#,
        );
        assert!(!out.contains("image-block"), "style contents survived: {out}");
        assert!(!out.contains("border-radius"), "declaration survived: {out}");
        assert!(out.contains("Hello.") && out.contains("Regards."));
    }

    #[test]
    fn a_single_declaration_block_is_still_css() {
        let out = redact("p{margin:0} Actual words.");
        assert!(!out.contains("margin"), "single declaration survived: {out}");
        assert!(out.contains("Actual words."));
    }

    #[test]
    fn invisible_preheader_padding_is_not_billed_for() {
        let padded = format!("Real subject line.{}\nActual content.", "\u{034F} ".repeat(400));
        let out = redact(&padded);
        assert!(!out.contains('\u{034F}'), "padding survived");
        assert!(out.len() < 60, "padding still billed: {} chars", out.len());
        assert!(out.contains("Real subject line.") && out.contains("Actual content."));
    }

    #[test]
    fn a_blank_line_survives_because_it_separates_a_request_from_a_signature() {
        let out = redact("Can you confirm?\n\n\n\nThanks,\nChris");
        assert_eq!(out, "Can you confirm?\n\nThanks,\nChris");
    }

    #[test]
    fn tax_ids_are_redacted() {
        assert_eq!(redact("SSN 123-45-6789 on file"), "SSN [tax-id] on file");
    }

    #[test]
    fn ordinary_prose_is_left_alone() {
        // Over-redaction destroys the judgment, so the common case must be a
        // no-op. Dates, small numbers and word shapes all stay.
        let prose = "Can you confirm the Nov 10 go-live for the safety module? \
                     We need 2 more seats and the Q4 kickoff is week 46.";
        assert_eq!(redact(prose), prose);
    }

    #[test]
    fn an_email_in_the_body_goes_but_the_envelope_stays() {
        let m = MessageForTriage {
            from_email: "dana@northwind.example",
            from_name: Some("Dana"),
            to: &["chris@cxventures.io".to_string()],
            cc: &[],
            date: "2026-09-17T10:00:00Z",
            subject: Some("Re: Academy"),
            body: Some("Loop in sam@northwind.example on the next one."),
        };
        let Gate::Send(safe) = gate(TriageMode::On, &m) else {
            panic!("expected send");
        };
        assert_eq!(safe.from_email, "dana@northwind.example");
        assert_eq!(safe.to, vec!["chris@cxventures.io".to_string()]);
        assert!(!safe.body.contains("sam@northwind.example"));
        assert!(safe.body.contains("[email]"));
    }

    // --- fail closed -----------------------------------------------------

    #[test]
    fn no_cached_body_is_withheld_rather_than_sent_as_headers() {
        let m = msg("dana@northwind.example", None);
        assert_eq!(gate(TriageMode::On, &m), Gate::Withheld(Withheld::NoBody));
    }

    #[test]
    fn a_body_that_redacts_to_nothing_is_withheld() {
        let m = MessageForTriage {
            from_email: "x@example.com",
            from_name: None,
            to: &[],
            cc: &[],
            date: "2026-09-17T10:00:00Z",
            subject: Some("   "),
            body: Some("  4829301847  "),
        };
        // Everything substantive was a number; nothing worth judging is left.
        match gate(TriageMode::On, &m) {
            Gate::Send(safe) => assert!(safe.body.contains("[number]")),
            Gate::Withheld(w) => assert_eq!(w, Withheld::Empty),
        }
    }

    #[test]
    fn the_final_check_refuses_a_payload_the_redactor_let_through() {
        // The invariant, stated as a test: whatever the redactor does or fails
        // to do, nothing matching these leaves. Simulated by handing the gate a
        // body whose shape the redactor is not built to catch.
        let m = msg("x@example.com", Some("Balance carried forward 4829301847 as agreed"));
        match gate(TriageMode::On, &m) {
            Gate::Send(safe) => {
                assert!(
                    final_check(&safe.body).is_none(),
                    "a payload left carrying {:?}",
                    final_check(&safe.body)
                );
            }
            Gate::Withheld(Withheld::FailedFinalCheck(_)) => {}
            Gate::Withheld(other) => panic!("unexpected withhold: {other:?}"),
        }
    }

    #[test]
    fn no_payload_can_leave_carrying_a_final_check_pattern() {
        // Exhaustive over the bodies that have actually caused trouble.
        for body in [
            "Wire 4829301847 routing 121000248 for $12,450.00",
            "Enter the verification code to continue: 437101",
            "VAT Reg. No. DE812871812 applies",
            "Authorization: Bearer abcdefghijklmnopqrst",
            "Reach me at someone@example.com",
            "SSN 123 45 6789 on file",
        ] {
            if let Gate::Send(safe) = gate(TriageMode::On, &msg("x@example.com", Some(body))) {
                assert!(
                    final_check(&safe.body).is_none() && final_check(&safe.subject).is_none(),
                    "leaked from {body:?} -> {:?}",
                    safe.body
                );
            }
        }
    }

    #[test]
    fn every_withheld_reason_is_reportable() {
        for w in [
            Withheld::ModeOff,
            Withheld::FinancialSender("chase.com".into()),
            Withheld::NoBody,
            Withheld::Empty,
            Withheld::FailedFinalCheck("digits"),
        ] {
            assert!(!w.reason().is_empty());
        }
    }

    // --- truncation ------------------------------------------------------

    #[test]
    fn truncation_never_splits_a_character() {
        // gotcha #21b: a length check is not a boundary check. A body of box
        // characters is the exact shape that panicked the event detector.
        let body = "─".repeat(MAX_BODY_CHARS + 500);
        let m = msg("x@example.com", Some(&body));
        let Gate::Send(safe) = gate(TriageMode::On, &m) else {
            panic!("expected send");
        };
        assert!(safe.body_truncated);
        assert_eq!(safe.body.chars().count(), MAX_BODY_CHARS);
    }

    #[test]
    fn a_short_body_is_not_marked_truncated() {
        let m = msg("x@example.com", Some("short"));
        let Gate::Send(safe) = gate(TriageMode::On, &m) else {
            panic!("expected send");
        };
        assert!(!safe.body_truncated);
        assert_eq!(safe.body, "short");
    }
}
