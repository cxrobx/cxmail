// Fiction-only browser acceptance: actual network requests, iframe permissions,
// trusted loading, failed authentication, one-time consent and revocation.
import assert from "node:assert/strict";
const { chromium } = await import(process.env.CXMAIL_PLAYWRIGHT_MODULE || "playwright");
const browser = await chromium.launch({ headless: true, ...(process.env.CXMAIL_BROWSER_EXECUTABLE ? { executablePath: process.env.CXMAIL_BROWSER_EXECUTABLE } : {}) });
try {
  const page = await browser.newPage({ viewport: { width: 1100, height: 820 } });
  const images = [];
  const errors = [];
  page.on("pageerror", e => errors.push(e.message));
  await page.route("https://images.example.org/**", route => {
    images.push(route.request().url());
    return route.fulfill({ status: 200, contentType: "image/svg+xml", body: '<svg xmlns="http://www.w3.org/2000/svg" width="480" height="180"><rect width="480" height="180" rx="12" fill="#1d4ed8"/><text x="28" y="100" font-family="sans-serif" font-size="32" fill="white">WEEKLY NOTES</text></svg>' });
  });
  const frame = page.frameLocator('iframe[title="Email content"]');
  const ready = async () => { await frame.locator("#remote-background").waitFor(); await page.waitForFunction(() => document.querySelector('iframe[title="Email content"]').style.height !== '200px'); };
  await page.goto(`${process.env.CXMAIL_TEST_URL || "http://127.0.0.1:5173"}/image-policy.html`);
  await page.getByRole("button", { name: "Always trust this sender" }).waitFor();
  await ready();
  assert.equal(images.length, 0, "Both img and CSS background must be blocked");
  await page.screenshot({ path: "/private/tmp/cxmail-images-blocked.png" });
  await page.getByRole("button", { name: "Always trust this sender" }).click();
  await page.waitForFunction(() => ![...document.querySelectorAll('button')].some(b => b.textContent.trim() === 'Show images'));
  await ready();
  await page.waitForFunction(() => document.querySelector('iframe').contentWindow !== null);
  await frame.locator("#remote-photo").evaluate(img => img.decode());
  assert(images.some(u => u.endsWith("photo.png")), "Trusted photo was requested");
  assert(images.some(u => u.endsWith("background.png")), "Trusted CSS image was requested");
  assert.equal(await page.locator("iframe").getAttribute("sandbox"), "allow-scripts allow-top-navigation-by-user-activation");
  await page.screenshot({ path: "/private/tmp/cxmail-images-trusted.png" });
  await page.getByRole("button", { name: "Toggle authentication" }).click();
  await page.getByRole("button", { name: "Show images" }).waitFor();
  await ready();
  let count = images.length;
  assert(!await frame.locator("#remote-photo").getAttribute("src"), "Failed authentication must restore blocked placeholders");
  await page.getByRole("button", { name: "Show images" }).click();
  await frame.locator("#remote-photo").evaluate(img => img.decode());
  assert(images.length > count, "Explicit one-time consent remains possible");
  await page.getByRole("button", { name: "Next message" }).click();
  await page.getByRole("button", { name: "Show images" }).waitFor();
  await ready();
  assert(!await frame.locator("#remote-photo").getAttribute("src"), "One-time consent must not follow a different UID with identical HTML");
  await page.getByRole("button", { name: "Toggle authentication" }).click();
  await frame.locator("#remote-photo").evaluate(img => img.decode());
  await page.getByRole("button", { name: "Revoke sender trust" }).click();
  await page.getByRole("button", { name: "Show images" }).waitFor();
  await ready();
  assert(!await frame.locator("#remote-photo").getAttribute("src"), "Revocation must remove automatic permission");
  assert.deepEqual(errors, []);
  console.log("PASS: network blocking, trusted images, CSS images, failed auth, one-time scope, revocation and sandbox.");
} finally { await browser.close(); }
