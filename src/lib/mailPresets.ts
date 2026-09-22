/**
 * Server presets for the generic IMAP provider — the TypeScript mirror of
 * `src-tauri/src/email/providers.rs::PRESETS`.
 *
 * ⚠️ These two tables are duplicated deliberately (same call as
 * `mailRuleValidation.ts` / gotcha #36: the UI validates in TS, the backend in
 * Rust) and **must be changed together**. The backend is authoritative — it
 * re-resolves the preset by id or domain and ignores anything the UI got
 * wrong — so drift shows up as a form that prefills the wrong host, not as a
 * wrong connection. A Rust test (`presets::ts_mirror_matches_the_rust_table`)
 * parses this file and fails if the two disagree.
 */
export interface MailPreset {
  id: string;
  label: string;
  imap_host: string;
  imap_port: number;
  imap_security: string;
  smtp_host: string;
  smtp_port: number;
  smtp_security: string;
  domains: string[];
  hint: string;
}

export const MAIL_PRESETS: MailPreset[] = [
  {
    id: "gmail",
    label: "Gmail (app password)",
    imap_host: "imap.gmail.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.gmail.com",
    smtp_port: 587,
    smtp_security: "starttls",
    domains: ["gmail.com", "googlemail.com"],
    hint: "Requires 2-Step Verification, then an App Password from myaccount.google.com.",
  },
  {
    id: "fastmail",
    label: "Fastmail",
    imap_host: "imap.fastmail.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.fastmail.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["fastmail.com", "fastmail.fm", "fastmail.us", "messagingengine.com"],
    hint: "Create an app password under Settings → Privacy & Security → App Passwords.",
  },
  {
    id: "icloud",
    label: "iCloud Mail",
    imap_host: "imap.mail.me.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.mail.me.com",
    smtp_port: 587,
    smtp_security: "starttls",
    domains: ["icloud.com", "me.com", "mac.com"],
    hint: "Requires an app-specific password from appleid.apple.com.",
  },
  {
    id: "yahoo",
    label: "Yahoo Mail",
    imap_host: "imap.mail.yahoo.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.mail.yahoo.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["yahoo.com", "yahoo.co.uk", "ymail.com", "rocketmail.com"],
    hint: "Generate an app password under Account Security.",
  },
  {
    id: "aol",
    label: "AOL Mail",
    imap_host: "imap.aol.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.aol.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["aol.com"],
    hint: "Generate an app password under Account Security.",
  },
  {
    id: "zoho",
    label: "Zoho Mail",
    imap_host: "imap.zoho.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.zoho.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["zoho.com", "zohomail.com"],
    hint: "Enable IMAP in Zoho settings, then create an app-specific password.",
  },
  {
    id: "gmx",
    label: "GMX",
    imap_host: "imap.gmx.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "mail.gmx.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["gmx.com", "gmx.net", "gmx.de", "gmx.co.uk"],
    hint: "Enable IMAP access in GMX settings first.",
  },
  {
    id: "mailcom",
    label: "mail.com",
    imap_host: "imap.mail.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.mail.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["mail.com", "email.com", "usa.com"],
    hint: "Enable IMAP access in mail.com settings first.",
  },
  {
    id: "yandex",
    label: "Yandex Mail",
    imap_host: "imap.yandex.com",
    imap_port: 993,
    imap_security: "implicit",
    smtp_host: "smtp.yandex.com",
    smtp_port: 465,
    smtp_security: "implicit",
    domains: ["yandex.com", "yandex.ru", "ya.ru"],
    hint: "Create an app password in Yandex ID → Security.",
  },
];

/**
 * Find the preset for an email address by its domain.
 *
 * `null` is the "Other" case — the form must then ask for server settings
 * rather than prefill a guess.
 */
export function presetForEmail(email: string): MailPreset | null {
  const at = email.lastIndexOf("@");
  if (at < 0) return null;
  const domain = email.slice(at + 1).trim().toLowerCase();
  if (!domain) return null;
  return MAIL_PRESETS.find((p) => p.domains.includes(domain)) ?? null;
}

export function presetById(id: string): MailPreset | null {
  return MAIL_PRESETS.find((p) => p.id === id) ?? null;
}
