use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum EmailCategory {
    Primary,
    Updates,
    Social,
    Promotions,
    Junk,
}

impl EmailCategory {
    pub fn as_str(&self) -> &'static str {
        match self {
            EmailCategory::Primary => "primary",
            EmailCategory::Updates => "updates",
            EmailCategory::Social => "social",
            EmailCategory::Promotions => "promotions",
            EmailCategory::Junk => "junk",
        }
    }
}

pub struct ClassificationInput<'a> {
    pub from_email: &'a str,
    pub from_name: Option<&'a str>,
    pub subject: Option<&'a str>,
    pub has_list_unsubscribe: bool,
    pub precedence: Option<&'a str>,
}

/// Classify an email into a category using heuristic rules.
/// Returns (category, confidence 0.0-1.0).
pub fn classify(input: &ClassificationInput) -> (EmailCategory, f32) {
    let from_lower = input.from_email.to_lowercase();
    let domain = from_lower.split('@').nth(1).unwrap_or("");
    let local_part = from_lower.split('@').next().unwrap_or("");

    // 0. Junk: gibberish local-part on a free-mail provider.
    // Targets keyboard-mash personal addresses (e.g. hejdididjfhdieifjhf@gmail.com).
    // Restricted to free-mail domains so legit auto-generated corporate addresses pass.
    if is_freemail_domain(domain) && is_gibberish_local_part(local_part) {
        return (EmailCategory::Junk, 0.95);
    }

    // 1. Known social network domains
    if is_social_domain(domain) {
        return (EmailCategory::Social, 0.95);
    }

    // 2. Known marketing/ESP domains
    if is_marketing_platform(domain) {
        return (EmailCategory::Promotions, 0.9);
    }

    // 3. Sender address patterns
    if let Some(cat) = classify_by_sender_pattern(local_part) {
        let confidence = if input.has_list_unsubscribe { 0.85 } else { 0.7 };
        return (cat, confidence);
    }

    // 4. Header signals
    if let Some(prec) = input.precedence {
        let prec_lower = prec.to_lowercase();
        if prec_lower == "bulk" || prec_lower == "junk" {
            if input.has_list_unsubscribe {
                return (EmailCategory::Promotions, 0.8);
            }
            return (EmailCategory::Updates, 0.7);
        }
        if prec_lower == "list" {
            return (EmailCategory::Updates, 0.65);
        }
    }

    // 5. List-Unsubscribe without other signals = likely promotions or updates
    if input.has_list_unsubscribe {
        if let Some(subject) = input.subject {
            let subj_lower = subject.to_lowercase();
            if has_promotional_keywords(&subj_lower) {
                return (EmailCategory::Promotions, 0.8);
            }
            if has_social_keywords(&subj_lower) {
                return (EmailCategory::Social, 0.7);
            }
        }
        return (EmailCategory::Updates, 0.6);
    }

    // 6. Subject line patterns (no header signals)
    if let Some(subject) = input.subject {
        let subj_lower = subject.to_lowercase();
        if has_update_keywords(&subj_lower) {
            return (EmailCategory::Updates, 0.6);
        }
        if has_promotional_keywords(&subj_lower) {
            return (EmailCategory::Promotions, 0.55);
        }
        if has_social_keywords(&subj_lower) {
            return (EmailCategory::Social, 0.55);
        }
    }

    // 7. Default: Primary
    (EmailCategory::Primary, 0.5)
}

fn is_social_domain(domain: &str) -> bool {
    // Strip subdomains for matching (e.g., mail.facebook.com -> facebook.com)
    let base = base_domain(domain);
    matches!(
        base,
        "facebook.com"
            | "facebookmail.com"
            | "twitter.com"
            | "x.com"
            | "linkedin.com"
            | "instagram.com"
            | "tiktok.com"
            | "reddit.com"
            | "discord.com"
            | "threads.net"
            | "pinterest.com"
            | "snapchat.com"
            | "tumblr.com"
            | "medium.com"
            | "quora.com"
            | "nextdoor.com"
            | "meetup.com"
            | "strava.com"
    )
}

fn is_marketing_platform(domain: &str) -> bool {
    let base = base_domain(domain);
    matches!(
        base,
        "mailchimp.com"
            | "sendgrid.net"
            | "constantcontact.com"
            | "hubspot.com"
            | "mailgun.com"
            | "amazonses.com"
            | "mandrillapp.com"
            | "sendinblue.com"
            | "brevo.com"
            | "klaviyo.com"
            | "convertkit.com"
            | "beehiiv.com"
            | "substack.com"
            | "mailerlite.com"
            | "campaignmonitor.com"
    )
}

fn classify_by_sender_pattern(local_part: &str) -> Option<EmailCategory> {
    // Notification/update patterns -> Updates
    let update_patterns = [
        "noreply",
        "no-reply",
        "no_reply",
        "donotreply",
        "do-not-reply",
        "notifications",
        "notification",
        "alerts",
        "alert",
        "updates",
        "update",
        "info",
        "support",
        "billing",
        "receipts",
        "orders",
        "shipping",
        "confirm",
        "verify",
        "security",
        "account",
        "service",
        "mailer-daemon",
        "postmaster",
    ];
    for pat in &update_patterns {
        if local_part == *pat || local_part.starts_with(&format!("{}.", pat)) {
            return Some(EmailCategory::Updates);
        }
    }

    // Promotional patterns -> Promotions
    let promo_patterns = [
        "marketing",
        "promo",
        "promotions",
        "deals",
        "offers",
        "newsletter",
        "news",
        "digest",
        "weekly",
        "daily",
        "campaign",
        "sale",
        "store",
        "shop",
    ];
    for pat in &promo_patterns {
        if local_part == *pat || local_part.starts_with(&format!("{}.", pat)) {
            return Some(EmailCategory::Promotions);
        }
    }

    None
}

fn has_promotional_keywords(subject: &str) -> bool {
    let keywords = [
        "% off",
        "sale",
        "deal",
        "limited time",
        "exclusive offer",
        "coupon",
        "discount",
        "free shipping",
        "buy now",
        "shop now",
        "don't miss",
        "last chance",
        "flash sale",
        "clearance",
        "save up to",
        "special offer",
        "unsubscribe",
    ];
    keywords.iter().any(|k| subject.contains(k))
}

fn has_social_keywords(subject: &str) -> bool {
    let keywords = [
        "commented on",
        "liked your",
        "mentioned you",
        "tagged you",
        "friend request",
        "new follower",
        "connected with",
        "invitation to connect",
        "replied to your",
        "shared a post",
        "new message from",
        "sent you a message",
        "wants to connect",
        "started following",
        "reacted to",
    ];
    keywords.iter().any(|k| subject.contains(k))
}

fn has_update_keywords(subject: &str) -> bool {
    let keywords = [
        "your order",
        "order confirmation",
        "shipping confirmation",
        "delivery update",
        "password reset",
        "verify your",
        "account activity",
        "receipt for",
        "payment received",
        "payment confirmation",
        "your invoice",
        "subscription",
        "renewal",
        "security alert",
        "sign-in",
        "login attempt",
        "two-factor",
        "verification code",
    ];
    keywords.iter().any(|k| subject.contains(k))
}

/// True if the domain is a major free-mail provider. Gibberish detection is
/// only applied to these, so legit auto-generated addresses on real domains
/// (e.g. `k8r2p9m@company.com`) aren't flagged.
fn is_freemail_domain(domain: &str) -> bool {
    let base = base_domain(domain);
    matches!(
        base,
        "gmail.com"
            | "googlemail.com"
            | "outlook.com"
            | "hotmail.com"
            | "live.com"
            | "msn.com"
            | "yahoo.com"
            | "yahoo.co.uk"
            | "ymail.com"
            | "rocketmail.com"
            | "aol.com"
            | "icloud.com"
            | "me.com"
            | "mac.com"
            | "proton.me"
            | "protonmail.com"
            | "pm.me"
            | "gmx.com"
            | "gmx.net"
            | "gmx.de"
            | "mail.com"
            | "mail.ru"
            | "zoho.com"
            | "yandex.com"
            | "yandex.ru"
            | "fastmail.com"
            | "tutanota.com"
            | "tuta.io"
    )
}

/// Heuristic keyboard-mash detector for email local-parts.
///
/// Trips when the local-part is "obviously random" — the typical shape of
/// throwaway spam senders (e.g. `hejdididjfhdieifjhf`). Requires at least
/// two independent signals to minimise false positives on legit short or
/// acronym-style handles.
pub fn is_gibberish_local_part(local_part: &str) -> bool {
    // Strip common separators people actually use in handles.
    let stripped: String = local_part
        .chars()
        .filter(|c| c.is_ascii_alphabetic())
        .collect();
    let len = stripped.len();

    // Too short to judge; real names and handles below this length are
    // very hard to classify reliably and we'd rather miss than false-positive.
    if len < 12 {
        return false;
    }

    let chars: Vec<char> = stripped.chars().collect();
    let vowels = "aeiou";

    // Signal A: vowel ratio outside the 20–70 % band typical of real names.
    let vowel_count = chars.iter().filter(|c| vowels.contains(**c)).count();
    let vowel_ratio = vowel_count as f32 / len as f32;
    let low_vowels = vowel_ratio < 0.20;
    let high_vowels = vowel_ratio > 0.70;

    // Signal B: consonant run ≥ 5 (e.g. "jhfdi", "djfhd"). Very rare in
    // real names; the strongest single indicator of keyboard mashing.
    let mut max_consonant_run = 0usize;
    let mut run = 0usize;
    for c in &chars {
        if !vowels.contains(*c) {
            run += 1;
            if run > max_consonant_run {
                max_consonant_run = run;
            }
        } else {
            run = 0;
        }
    }
    let long_consonant_run = max_consonant_run >= 5;

    // Signal C: average English bigram plausibility. Penalise rare/impossible
    // pairs like "jh", "fh", "dj" that never appear in real English words.
    let bigram_score = english_bigram_score(&chars);
    let implausible_bigrams = bigram_score < 0.45;

    // Signal D: low unique-character diversity relative to length. Catches
    // the other common shape — long local-parts that reuse the same few
    // characters over and over.
    let mut seen = [false; 26];
    for c in &chars {
        let idx = (*c as u8 - b'a') as usize;
        if idx < 26 {
            seen[idx] = true;
        }
    }
    let unique = seen.iter().filter(|b| **b).count();
    let low_diversity = (unique as f32 / len as f32) < 0.50;

    // Require at least two signals. Bigram plausibility is the strongest so
    // it can team up with any single secondary.
    let signal_count = [
        low_vowels,
        high_vowels,
        long_consonant_run,
        implausible_bigrams,
        low_diversity,
    ]
    .iter()
    .filter(|b| **b)
    .count();

    signal_count >= 2
}

/// Rough English bigram plausibility score in [0.0, 1.0]. Uses a small
/// whitelist of common bigrams — not a full language model, just enough
/// to distinguish real words from keyboard mashing.
fn english_bigram_score(chars: &[char]) -> f32 {
    if chars.len() < 2 {
        return 1.0;
    }
    // Top English bigrams — covers ~60% of real text. Non-exhaustive but
    // sufficient for a "does this look like language" check.
    const COMMON: &[&str] = &[
        "th", "he", "in", "er", "an", "re", "on", "at", "en", "nd", "ti", "es", "or", "te", "of",
        "ed", "is", "it", "al", "ar", "st", "to", "nt", "ng", "se", "ha", "as", "ou", "io", "le",
        "ve", "co", "me", "de", "hi", "ri", "ro", "ic", "ne", "ea", "ra", "ce", "li", "ch", "ll",
        "be", "ma", "si", "om", "ur", "ca", "el", "ta", "la", "ns", "di", "fo", "ho", "pe", "ec",
        "pr", "no", "ct", "us", "ac", "ot", "il", "tr", "ly", "nc", "et", "ut", "ss", "so", "rs",
        "un", "lo", "wa", "ge", "ie", "wh", "ee", "wi", "em", "ad", "ol", "rt", "po", "we", "na",
        "ul", "ni", "ts", "mo", "ow", "pa", "im", "mi", "ai", "sh", "ir", "su", "id", "os", "iv",
        "ia", "am", "fi", "ci", "vi", "pl", "ig", "tu", "ev", "ld", "ry", "mp", "fe", "bl", "ab",
        "gh", "ty", "op", "wo", "sa", "ay", "ex", "ke", "fr", "oo", "av", "ag", "if", "ap", "gr",
        "od", "bo", "sp", "rd", "do", "uc", "bu", "ei", "ov", "by", "rm", "ep", "tt", "oc", "fa",
        "ef", "cu", "rn", "sc", "gi", "da", "yo", "cr", "cl", "du", "ga", "qu", "ue", "ff", "ba",
        "ey", "ls", "va", "um", "pp", "ua", "up", "lu", "go", "ht", "ru", "ug", "ds", "lt", "pi",
        "rc", "rr", "eg", "au", "ck", "ew", "mu", "br", "bi", "pt", "ak", "pu", "ui", "rg", "ib",
        "tl", "ny", "ki", "rk", "ys", "ob", "mm", "fu", "ph", "og", "ms", "ca", "nn",
    ];
    let mut hits = 0usize;
    let total = chars.len() - 1;
    for pair in chars.windows(2) {
        let bg: String = pair.iter().collect();
        if COMMON.contains(&bg.as_str()) {
            hits += 1;
        }
    }
    hits as f32 / total as f32
}

/// Extract base domain (strip subdomains). e.g., "mail.facebook.com" -> "facebook.com"
fn base_domain(domain: &str) -> &str {
    let parts: Vec<&str> = domain.split('.').collect();
    if parts.len() >= 2 {
        // Handle two-part TLDs like .co.uk
        let last = parts[parts.len() - 1];
        let second_last = parts[parts.len() - 2];
        if (last == "uk" || last == "au" || last == "jp") && second_last == "co" {
            if parts.len() >= 3 {
                let start = domain.len()
                    - last.len()
                    - 1
                    - second_last.len()
                    - 1
                    - parts[parts.len() - 3].len();
                return &domain[start..];
            }
        }
        // Standard: last two parts
        let start = domain.len() - last.len() - 1 - second_last.len();
        &domain[start..]
    } else {
        domain
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_social_classification() {
        let input = ClassificationInput {
            from_email: "notifications@facebook.com",
            from_name: Some("Facebook"),
            subject: Some("John commented on your post"),
            has_list_unsubscribe: true,
            precedence: None,
        };
        let (cat, conf) = classify(&input);
        assert_eq!(cat, EmailCategory::Social);
        assert!(conf > 0.9);
    }

    #[test]
    fn test_promotional_classification() {
        let input = ClassificationInput {
            from_email: "deals@store.mailchimp.com",
            from_name: Some("Big Store"),
            subject: Some("50% off everything - limited time!"),
            has_list_unsubscribe: true,
            precedence: Some("bulk"),
        };
        let (cat, _) = classify(&input);
        assert_eq!(cat, EmailCategory::Promotions);
    }

    #[test]
    fn test_update_classification() {
        let input = ClassificationInput {
            from_email: "noreply@amazon.com",
            from_name: Some("Amazon"),
            subject: Some("Your order has shipped"),
            has_list_unsubscribe: false,
            precedence: None,
        };
        let (cat, _) = classify(&input);
        assert_eq!(cat, EmailCategory::Updates);
    }

    #[test]
    fn test_primary_classification() {
        let input = ClassificationInput {
            from_email: "john@gmail.com",
            from_name: Some("John Doe"),
            subject: Some("Hey, are you free for lunch?"),
            has_list_unsubscribe: false,
            precedence: None,
        };
        let (cat, _) = classify(&input);
        assert_eq!(cat, EmailCategory::Primary);
    }

    #[test]
    fn test_base_domain() {
        assert_eq!(base_domain("facebook.com"), "facebook.com");
        assert_eq!(base_domain("mail.facebook.com"), "facebook.com");
        assert_eq!(base_domain("bounce.mail.facebook.com"), "facebook.com");
    }

    #[test]
    fn test_gibberish_flags_keyboard_mash() {
        // The example from the bug report.
        assert!(is_gibberish_local_part("hejdididjfhdieifjhf"));
        // Other obvious keyboard-mash patterns.
        assert!(is_gibberish_local_part("xjkdlwpqzvfnbmcxz"));
        assert!(is_gibberish_local_part("qwertyqwertyqw"));
    }

    #[test]
    fn test_gibberish_allows_real_names() {
        // Short handles are always allowed.
        assert!(!is_gibberish_local_part("chris"));
        assert!(!is_gibberish_local_part("jsmith"));
        assert!(!is_gibberish_local_part("john.doe"));
        // Long but real names.
        assert!(!is_gibberish_local_part("christopherrobinson"));
        assert!(!is_gibberish_local_part("johnmichaelsmith"));
        assert!(!is_gibberish_local_part("sarahjohnson"));
        // Auto-generated corporate addresses with a few digits — stripped
        // of digits these are still short enough to be skipped.
        assert!(!is_gibberish_local_part("jsmith1985"));
    }

    #[test]
    fn test_junk_classification() {
        let input = ClassificationInput {
            from_email: "hejdididjfhdieifjhf@gmail.com",
            from_name: Some("Rakesh"),
            subject: None,
            has_list_unsubscribe: false,
            precedence: None,
        };
        let (cat, conf) = classify(&input);
        assert_eq!(cat, EmailCategory::Junk);
        assert!(conf > 0.9);
    }

    #[test]
    fn test_junk_skips_corporate_domains() {
        // A short random-looking local-part on a real corporate domain
        // should NOT be flagged — under-12-char guard + freemail-only gate.
        let input = ClassificationInput {
            from_email: "k8r2p9m4qv@company.com",
            from_name: None,
            subject: Some("Order confirmation"),
            has_list_unsubscribe: false,
            precedence: None,
        };
        let (cat, _) = classify(&input);
        assert_ne!(cat, EmailCategory::Junk);
    }

    #[test]
    fn test_junk_real_name_on_freemail() {
        let input = ClassificationInput {
            from_email: "john.smith@gmail.com",
            from_name: Some("John Smith"),
            subject: Some("Lunch tomorrow?"),
            has_list_unsubscribe: false,
            precedence: None,
        };
        let (cat, _) = classify(&input);
        assert_eq!(cat, EmailCategory::Primary);
    }
}
