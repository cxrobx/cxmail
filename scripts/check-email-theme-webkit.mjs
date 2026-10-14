// Run against `npm run dev` with Playwright + WebKit installed.
// Optional: CXMAIL_PLAYWRIGHT_MODULE, CXMAIL_WEBKIT_EXECUTABLE, CXMAIL_TEST_URL,
// CXMAIL_EMAIL_THEME_SCREENSHOT. Uses synthetic mail and no connected accounts.
import assert from "node:assert/strict";
const { webkit } = await import(process.env.CXMAIL_PLAYWRIGHT_MODULE || "playwright");
const browser = await webkit.launch({
  headless: true,
  timeout: 10_000,
  ...(process.env.CXMAIL_WEBKIT_EXECUTABLE ? { executablePath: process.env.CXMAIL_WEBKIT_EXECUTABLE } : {}),
});
console.log("WebKit launched.");
try {
  const page = await browser.newPage({ viewport: { width: 1200, height: 1400 } });
  console.log("WebKit page created.");
  page.setDefaultTimeout(10_000);
  const errors = [];
  const remoteRequests = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.route("https://example.test/**", (route) => {
    remoteRequests.push(route.request().url());
    return route.abort();
  });
  await page.goto(`${process.env.CXMAIL_TEST_URL || "http://localhost:5173"}/email-theme.html`);
  console.log("Email contrast fixture loaded.");
  const content = page.frameLocator('iframe[title="Email content"]');
  await content.locator("#receipt-name").waitFor();
  await page.waitForFunction(() => parseInt(document.querySelector('iframe[title="Email content"]').style.height) > 200);

  async function checkContrast(frame) {
    const ratios = await frame.locator("body").evaluate((body) => {
      const parse = (css) => {
        const parts = css.match(/[\d.]+/g).map(Number);
        return [...parts.slice(0, 3), parts[3] ?? 1];
      };
      const over = (layer, ground) => layer.slice(0, 3).map((channel, i) => channel * layer[3] + ground[i] * (1 - layer[3]));
      const luminance = (rgb) => {
        const lin = rgb.map((channel) => {
          const value = channel / 255;
          return value <= 0.04045 ? value / 12.92 : ((value + 0.055) / 1.055) ** 2.4;
        });
        return lin[0] * .2126 + lin[1] * .7152 + lin[2] * .0722;
      };
      const ratios = {};
      for (const element of body.querySelectorAll("[id]")) {
        if (element.children.length || element.tagName === "IMG") continue;
        const ancestors = [];
        for (let node = element; node; node = node.parentElement) ancestors.unshift(node);
        let ground = [28, 26, 23];
        for (const ancestor of ancestors) ground = over(parse(getComputedStyle(ancestor).backgroundColor), ground);
        const ink = over(parse(getComputedStyle(element).color), ground);
        const [low, high] = [luminance(ink), luminance(ground)].sort((a, b) => a - b);
        ratios[element.id] = (high + .05) / (low + .05);
      }
      return ratios;
    });
    assert.equal(Object.keys(ratios).length, 21);
    for (const [id, ratio] of Object.entries(ratios)) {
      assert.ok(ratio >= 4.5, `${id} has ${ratio.toFixed(2)}:1 contrast; expected at least 4.5:1`);
    }
  }
  await checkContrast(content);
  assert.equal(await content.locator("#white-card").evaluate((element) => getComputedStyle(element).backgroundColor), "rgb(255, 255, 255)");
  assert.equal(await content.locator("#receipt-name").evaluate((element) => getComputedStyle(element).color), "rgb(34, 34, 34)");
  assert.equal(await content.locator("#receipt-status").evaluate((element) => getComputedStyle(element).color), "rgb(102, 102, 102)");
  assert.equal(await content.locator("#dark-card-text").evaluate((element) => getComputedStyle(element).color), "rgb(255, 255, 255)");
  assert.equal(await content.locator("#blue-button").evaluate((element) => getComputedStyle(element).backgroundColor), "rgb(0, 0, 128)");
  assert.equal(await content.locator("#currentcolor-background").evaluate((element) => getComputedStyle(element).backgroundColor), "rgb(255, 255, 255)");
  assert.equal(await content.getByTestId("hidden-preheader").evaluate((element) => getComputedStyle(element).color), "rgba(0, 0, 0, 0)");
  assert.equal(await content.locator("#blocked-image").getAttribute("src"), "");
  assert.equal(await content.locator("#blocked-image").evaluate((element) => getComputedStyle(element).display), "none");
  assert.equal(await page.locator('iframe[title="Email content"]').getAttribute("sandbox"), "allow-scripts allow-top-navigation-by-user-activation");

  const darkBody = await content.locator(".email-body").innerHTML();
  if (process.env.CXMAIL_EMAIL_THEME_SCREENSHOT) await page.screenshot({ path: process.env.CXMAIL_EMAIL_THEME_SCREENSHOT });
  await page.getByTestId("preview-row").hover();
  const preview = page.frameLocator('iframe[title="Email preview"]');
  await preview.locator("#receipt-name").waitFor();
  await checkContrast(preview);
  assert.equal(await page.locator('iframe[title="Email preview"]').getAttribute("sandbox"), "");
  assert.equal(await preview.locator("script").count(), 0);
  await page.getByRole("button", { name: "Light mode", exact: true }).click();
  await content.locator("#pale-on-white").waitFor();
  await page.waitForFunction(() => document.querySelector('iframe[title="Email content"]').srcdoc.includes('color:#e8dcc8!important'));
  assert.equal(await content.locator("#pale-on-white").evaluate((element) => getComputedStyle(element).color), "rgb(232, 220, 200)");
  assert.equal(await content.locator("#plain-dark").evaluate((element) => getComputedStyle(element).color), "rgb(0, 0, 0)");
  await page.getByRole("button", { name: "Dark mode", exact: true }).click();
  await page.waitForFunction(() => !document.querySelector('iframe[title="Email content"]').srcdoc.includes('color:#e8dcc8!important'));
  await content.locator("#receipt-name").waitFor();
  await checkContrast(content);
  assert.equal(await content.locator(".email-body").innerHTML(), darkBody, "theme flips must start from the original email");
  assert.deepEqual(remoteRequests, [], "contrast correction must never load blocked remote images");
  assert.deepEqual(errors, []);
  console.log("WebKit passed: receipt contrast, nested backgrounds, colored buttons, alpha, legacy markup, preview, theme switching and blocked images.");
} finally {
  await browser.close();
}
