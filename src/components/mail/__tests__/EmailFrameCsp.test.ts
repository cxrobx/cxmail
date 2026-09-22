// Scoped to this file on purpose: tsconfig's `types` allowlist deliberately keeps
// node globals out of the app's type surface, and this is the only test that reads
// from disk. Referencing here beats widening `types` for everything under src/.
/// <reference types="node" />
import { describe, it, expect } from "vitest";
import { readFileSync, existsSync } from "node:fs";
import path from "node:path";

/**
 * Regression guard for gotcha #29.
 *
 * The app CSP (tauri.conf.json) ends in `script-src 'self'` with no
 * `'unsafe-inline'`. A srcdoc iframe INHERITS the embedder's CSP, so the script
 * EmailFrame injects into the sandboxed email iframe must be an external file —
 * an inline <script> is silently blocked, no cxmail-frame-height message is ever
 * posted, and every email body renders clipped to the initial 200px frameHeight.
 *
 * This failure is invisible in a release build (no DevTools) and cost a full
 * debugging session once, so it is pinned here rather than left to review.
 *
 * Note a 'sha256-...' hash allowlist is NOT an escape hatch: WebKit (WKWebView,
 * the engine CXMail actually runs on) ignores hash allowances for inline scripts
 * in sandboxed srcdoc frames, even though Chromium honors them.
 */

const repoRoot = path.resolve(__dirname, "../../../..");
const emailFrameSrc = readFileSync(
  path.join(repoRoot, "src/components/mail/EmailFrame.tsx"),
  "utf8",
);

/**
 * The `<script ...>` tags EmailFrame injects into the iframe document.
 *
 * Read out of the FRAME_SCRIPT constant specifically, not the whole file —
 * prose in the surrounding comments mentions `<script>` too.
 */
function injectedScriptTags(): string[] {
  const frameScript = emailFrameSrc.match(/const FRAME_SCRIPT = `([\s\S]*?)`;/)?.[1];
  expect(frameScript, "could not find the FRAME_SCRIPT constant in EmailFrame.tsx").toBeTruthy();
  return frameScript!.match(/<script\b[^>]*>/g) ?? [];
}

describe("email iframe script vs app CSP", () => {
  it("app CSP still blocks inline scripts (the precondition this test exists for)", () => {
    const conf = JSON.parse(
      readFileSync(path.join(repoRoot, "src-tauri/tauri.conf.json"), "utf8"),
    );
    const csp: string | null = conf.app?.security?.csp ?? null;
    if (csp === null) return; // CSP disabled — inline would be fine, nothing to guard

    const scriptSrc = csp
      .split(";")
      .map((d: string) => d.trim())
      .find((d: string) => d.startsWith("script-src"));

    expect(scriptSrc, "CSP has no script-src directive").toBeTruthy();
    expect(
      scriptSrc,
      "script-src gained 'unsafe-inline' — if that was deliberate, this guard can relax; " +
        "if not, it silently weakened the app CSP",
    ).not.toContain("unsafe-inline");
  });

  it("injects the frame script by src, never inline", () => {
    const tags = injectedScriptTags();
    expect(tags.length, "expected exactly one injected <script> tag").toBe(1);
    expect(
      tags[0],
      "inline <script> in the srcdoc iframe is blocked by the inherited CSP — " +
        "keep the script in public/email-frame.js",
    ).toMatch(/\ssrc=/);
  });

  it("the referenced script file exists at the served path", () => {
    const src = injectedScriptTags()[0]?.match(/src="([^"]+)"/)?.[1];
    expect(src, "could not read src= off the injected script tag").toBeTruthy();
    expect(src!.startsWith("/"), "src must be an absolute app-origin path").toBe(true);

    // Vite copies public/ to the dist root unhashed, so "/x.js" is served from public/x.js.
    const onDisk = path.join(repoRoot, "public", src!.slice(1));
    expect(
      existsSync(onDisk),
      `${src} is referenced but public/${src!.slice(1)} does not exist`,
    ).toBe(true);
  });

  it("the frame script still posts the height message the parent listens for", () => {
    const script = readFileSync(path.join(repoRoot, "public/email-frame.js"), "utf8");
    expect(script).toContain("cxmail-frame-height");
    // The parent's message handler keys off these exact types.
    for (const type of [
      "cxmail-open-link",
      "cxmail-load-images",
      "cxmail-image-contextmenu",
      "cxmail-iframe-pointerdown",
      "cxmail-iframe-wheel",
    ]) {
      expect(script, `frame script no longer handles ${type}`).toContain(type);
    }
  });
});
