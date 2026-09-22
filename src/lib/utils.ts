import { clsx, type ClassValue } from "clsx";
import { twMerge } from "tailwind-merge";
import type { Editor } from "@tiptap/react";

export function cn(...inputs: ClassValue[]) {
  return twMerge(clsx(inputs));
}

export function escapeHtml(value: string): string {
  return value
    .replaceAll("&", "&amp;")
    .replaceAll("<", "&lt;")
    .replaceAll(">", "&gt;")
    .replaceAll('"', "&quot;")
    .replaceAll("'", "&#39;");
}

export function formatBytes(bytes: number): string {
  if (bytes < 1024) return `${bytes} B`;
  if (bytes < 1024 * 1024) return `${(bytes / 1024).toFixed(1)} KB`;
  if (bytes < 1024 * 1024 * 1024) return `${(bytes / (1024 * 1024)).toFixed(1)} MB`;
  return `${(bytes / (1024 * 1024 * 1024)).toFixed(1)} GB`;
}

export function formatRelativeDate(dateStr: string): string {
  const date = new Date(dateStr);
  const now = new Date();
  const diffMs = now.getTime() - date.getTime();
  const diffMins = Math.floor(diffMs / 60000);
  const diffHours = Math.floor(diffMs / 3600000);
  const diffDays = Math.floor(diffMs / 86400000);

  if (diffMins < 1) return "now";
  if (diffMins < 60) return `${diffMins}m`;
  if (diffHours < 24) return `${diffHours}h`;
  if (diffDays < 7) return `${diffDays}d`;

  // Anything outside the current year needs the year to be readable — "May 14"
  // on a 2024 message is indistinguishable from one sent this May.
  return date.toLocaleDateString("en-US", {
    month: "short",
    day: "numeric",
    ...(date.getFullYear() === now.getFullYear() ? {} : { year: "numeric" }),
  });
}

const ATTACHMENT_PATTERN =
  /\b(?:i(?:'ve|'m|\s+have|\s+am)\s+attach(?:ed|ing)|(?:please\s+)?(?:find|see)\s+(?:the\s+)?attach(?:ed|ments?)|attach(?:ed|ments?|ing)|enclos(?:ed|ing|ure))\b/i;

export function detectAttachmentMention(plainText: string): boolean {
  return ATTACHMENT_PATTERN.test(plainText);
}

const gravatarCache = new Map<string, string>();

export async function getGravatarUrl(email: string, size: number = 40): Promise<string> {
  const key = `${email.trim().toLowerCase()}-${size}`;
  const cached = gravatarCache.get(key);
  if (cached) return cached;

  const normalized = email.trim().toLowerCase();
  const data = new TextEncoder().encode(normalized);
  const hashBuffer = await crypto.subtle.digest("SHA-256", data);
  const hash = Array.from(new Uint8Array(hashBuffer))
    .map((b) => b.toString(16).padStart(2, "0"))
    .join("");
  const url = `https://gravatar.com/avatar/${hash}?s=${size}&d=404`;
  gravatarCache.set(key, url);
  return url;
}

export function getInitials(name: string | null, email: string | null): string {
  if (name) {
    const parts = name.split(" ");
    if (parts.length >= 2) {
      return (parts[0][0] + parts[parts.length - 1][0]).toUpperCase();
    }
    return name.substring(0, 2).toUpperCase();
  }
  if (email) {
    return email.substring(0, 2).toUpperCase();
  }
  return "??";
}

export function buildQuotedEmailHtml(detail: {
  from_name: string | null;
  from_email: string;
  date: string | null;
  sanitized_html: string | null;
  plain_text: string | null;
}): string {
  const author = escapeHtml(detail.from_name || detail.from_email || "Unknown sender");
  const date = escapeHtml(
    detail.date ? new Date(detail.date).toLocaleString() : "unknown date",
  );
  const quotedBody = detail.sanitized_html
    ? detail.sanitized_html
    : `<pre>${escapeHtml(detail.plain_text || "")}</pre>`;

  return `<br/><br/><div style="border-left:2px solid rgb(74, 68, 57);padding-left:12px;margin-left:4px;color:rgb(160, 144, 120);"><p><strong>${author}</strong> wrote on ${date}:</p>${quotedBody}</div>`;
}

export function stripSignatureBlocks(html: string): string {
  if (!html) return html;
  try {
    const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
    doc.body
      .querySelectorAll('div.email-signature, div[data-cx-signature="1"]')
      .forEach((n) => n.remove());
    return doc.body.innerHTML;
  } catch {
    return html;
  }
}

function extractSignatureHtml(html: string): string {
  if (!html) return "";
  try {
    const doc = new DOMParser().parseFromString(`<body>${html}</body>`, "text/html");
    const sig = doc.body.querySelector(
      'div[data-cx-signature="1"], div.email-signature',
    );
    return sig ? sig.outerHTML : "";
  } catch {
    return "";
  }
}

/// Convert plain text from the AI to HTML paragraphs that render with proper
/// spacing in TipTap and recipient inboxes. Mirrors `body_to_html` in
/// `src-tauri/src/mcp/server.rs` — see gotcha #13 for why a single `<p>` with
/// `<br>` between paragraphs collapses spacing.
export function plainToHtmlParagraphs(plain: string): string {
  const paragraphs = plain.replace(/\r\n/g, "\n").split(/\n{2,}/);
  return paragraphs
    .map((p) => p.trim())
    .filter((p) => p.length > 0)
    .map((p) => {
      const withBreaks = escapeHtml(p).replace(/\n/g, "<br/>");
      return `<p>${withBreaks}</p>`;
    })
    .join("");
}

export function setEditorContent(editor: Editor | null, html: string): void {
  if (!editor) return;
  const preservedSignature = extractSignatureHtml(editor.getHTML());
  const stripped = stripSignatureBlocks(html);
  editor.commands.setContent(stripped + preservedSignature);
}
