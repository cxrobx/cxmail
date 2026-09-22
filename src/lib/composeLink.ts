/**
 * Turn what the user typed into the link field into an href, or null when it
 * cannot be one.
 *
 * - no scheme and shaped like an address → `mailto:`
 * - no scheme otherwise → `https://` (people type `example.com`, not the scheme)
 * - an explicit scheme must be one a recipient's mail client can safely follow;
 *   `javascript:` and friends are refused rather than rewritten.
 */
const ALLOWED_SCHEMES = new Set(["http", "https", "mailto", "tel"]);

export function normalizeLinkHref(input: string): string | null {
  const value = input.trim();
  if (!value || /\s/.test(value)) return null;

  const scheme = /^([a-z][a-z0-9+.-]*):/i.exec(value);
  // `localhost:3000` parses as a scheme; a scheme is only real when followed by
  // something other than a bare port number.
  if (scheme && !/^[^:]+:\d+(\/|$)/.test(value)) {
    return ALLOWED_SCHEMES.has(scheme[1].toLowerCase()) ? value : null;
  }

  if (/^[^@/]+@[^@/]+\.[^@/]+$/.test(value)) return `mailto:${value}`;
  return `https://${value.replace(/^\/+/, "")}`;
}
