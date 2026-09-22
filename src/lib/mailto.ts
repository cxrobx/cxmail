// RFC 6068 `mailto:` URI parser.
//
// Used by the deep-link funnel in AppLayout to turn an incoming `mailto:` URL
// (from the OS as the default handler, or from an in-app link) into a prefilled
// compose. This supersedes the Rust `parse_mailto` (to/subject/body only) for
// the deep-link path — the Rust helper is left untouched because `unsubscribe()`
// still depends on it.
//
// Grammar handled:
//   mailto:alice@x.com,bob@y.com?cc=c@z.com&bcc=d@z.com&subject=Hi&body=Hello
//   mailto:?to=alice@x.com&subject=...
//
// Notes:
// - Path recipients and `to=`/`cc=`/`bcc=` values are comma-split.
// - All components are %-decoded with decodeURIComponent. Per RFC 6068 a literal
//   "+" is NOT a space in a mailto URI (unlike application/x-www-form-urlencoded),
//   and decodeURIComponent already leaves "+" intact — so no "+"→space step.

export interface ParsedMailto {
  to: string[];
  cc: string[];
  bcc: string[];
  subject?: string;
  body?: string;
}

function decode(s: string): string {
  try {
    return decodeURIComponent(s);
  } catch {
    // Malformed percent-encoding — fall back to the raw value rather than throw.
    return s;
  }
}

function splitAddresses(s: string): string[] {
  return s
    .split(",")
    .map((a) => decode(a).trim())
    .filter(Boolean);
}

/**
 * Convert an RFC 6068 plain-text `body` into the HTML that ComposeModal's
 * `defaultBody` prop expects (everywhere else in the app, `defaultBody` is HTML).
 *
 * SECURITY: the body comes from an arbitrary `mailto:` URL — including links
 * inside *received* (hostile) email. It MUST be HTML-escaped before reaching the
 * TipTap editor: the editor parses its initial `content` as HTML in the
 * privileged main WebView, and a custom node (`div.cx-html-block`) would
 * `innerHTML =` raw markup, executing e.g. `<img onerror>` and breaking the
 * email sandbox (invariant #3). Escaping makes the payload literal text.
 * Newlines are preserved as `<br>` so multi-line bodies keep their formatting.
 */
export function plainBodyToComposeHtml(body: string): string {
  const escaped = body
    .replace(/&/g, "&amp;")
    .replace(/</g, "&lt;")
    .replace(/>/g, "&gt;")
    .replace(/\r\n?/g, "\n");
  return `<p>${escaped.replace(/\n/g, "<br>")}</p>`;
}

export function parseMailto(url: string): ParsedMailto {
  const result: ParsedMailto = { to: [], cc: [], bcc: [] };
  if (!url) return result;

  // Strip the scheme case-insensitively; tolerate an (uncommon) leading "//".
  const withoutScheme = url.replace(/^mailto:/i, "").replace(/^\/\//, "");

  const qIndex = withoutScheme.indexOf("?");
  const pathPart = qIndex === -1 ? withoutScheme : withoutScheme.slice(0, qIndex);
  const queryPart = qIndex === -1 ? "" : withoutScheme.slice(qIndex + 1);

  if (pathPart) result.to.push(...splitAddresses(pathPart));

  if (queryPart) {
    for (const pair of queryPart.split("&")) {
      if (!pair) continue;
      const eq = pair.indexOf("=");
      const key = (eq === -1 ? pair : pair.slice(0, eq)).toLowerCase();
      const rawVal = eq === -1 ? "" : pair.slice(eq + 1);
      switch (key) {
        case "to":
          result.to.push(...splitAddresses(rawVal));
          break;
        case "cc":
          result.cc.push(...splitAddresses(rawVal));
          break;
        case "bcc":
          result.bcc.push(...splitAddresses(rawVal));
          break;
        case "subject":
          result.subject = decode(rawVal);
          break;
        case "body":
          result.body = decode(rawVal);
          break;
        default:
          break;
      }
    }
  }

  return result;
}
