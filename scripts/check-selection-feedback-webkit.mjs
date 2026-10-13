// Run against `npm run dev` with Playwright + its WebKit browser installed.
// CXMAIL_PLAYWRIGHT_MODULE and CXMAIL_WEBKIT_EXECUTABLE can use a bundled runtime.
import assert from "node:assert/strict";
const { webkit } = await import(process.env.CXMAIL_PLAYWRIGHT_MODULE || "playwright");
const browser = await webkit.launch({
  headless: true,
  timeout: 10_000,
  ...(process.env.CXMAIL_WEBKIT_EXECUTABLE ? { executablePath: process.env.CXMAIL_WEBKIT_EXECUTABLE } : {}),
});
console.log("WebKit launched.");
try {
  const page = await browser.newPage({ viewport: { width: 1000, height: 900 } });
  page.setDefaultTimeout(10_000);
  const errors = [];
  page.on("pageerror", (error) => { errors.push(error.message); console.error(error.message); });
  await page.goto(`${process.env.CXMAIL_TEST_URL || "http://localhost:5173"}/selection-feedback.html`);
  console.log("WebKit loaded the selection feedback harness.");
  await page.waitForFunction(() => document.querySelector(".tiptap")?.editor?.isInitialized);
  const feedback = page.getByRole("dialog", { name: "Selection feedback" });
  const selectDraft = async (selector = ".tiptap > p", start = 0, end = 13) => {
    await page.locator(selector).first().evaluate((element, { start, end }) => {
      element.closest('[contenteditable="true"]')?.focus();
      const text = element.firstChild;
      const range = document.createRange(); range.setStart(text, start); range.setEnd(text, end);
      const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
      element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
    }, { start, end });
  };
  await selectDraft();
  await feedback.waitFor();
  assert.equal(await page.evaluate(() => getSelection().toString()), "Draft passage");
  assert.equal(await page.evaluate(() => document.activeElement.isContentEditable), true);
  assert.equal(await page.getByRole("button", { name: "Give Claude feedback on selection" }).count(), 0);
  await page.getByRole("textbox", { name: "Tell Claude what to change" }).fill("Make it natural");
  await page.getByRole("textbox", { name: "Tell Claude what to change" }).press("Enter");
  await page.waitForFunction(() => document.querySelector(".tiptap > p")?.textContent === "Natural version for feedback.");
  await page.getByRole("button", { name: "Undo", exact: true }).click();
  assert.equal(await page.locator(".tiptap > p").first().textContent(), "Draft passage for feedback.");
  for (const selector of [".cx-html-block p", ".email-signature p", ".cx-quote p"]) {
    await selectDraft(selector, 0, 8);
    assert.equal(await feedback.count(), 0, `${selector} must not offer draft replacement`);
  }

  await page.getByRole("button", { name: "Slow model" }).click();
  await selectDraft(); await feedback.waitFor();
  await page.getByRole("button", { name: "Shorten", exact: true }).click();
  // Edit the real document while inference is in flight without dismissing
  // the bubble; the asynchronous result must fail its saved-text check.
  await page.getByRole("button", { name: "Change draft" }).evaluate((button) => button.click());
  await page.getByText("Your draft changed. Copy this version to use it.").waitFor();
  assert.equal(await page.locator(".tiptap > p").first().textContent(), "changed Draft passage for feedback.");
  await page.getByRole("textbox", { name: "Tell Claude what to change" }).press("Escape");
  assert.equal(await page.getByRole("dialog").count(), 0);

  const iframe = page.locator('iframe[title="Email content"]');
  const frame = page.frameLocator('iframe[title="Email content"]');
  const selectMail = async () => {
    await frame.locator("p").evaluate((element) => {
      const range = document.createRange(); range.selectNodeContents(element);
      const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
      element.dispatchEvent(new PointerEvent("pointerup", { bubbles: true, button: 0 }));
    });
    await feedback.waitFor();
  };
  // The parent truly cannot read the frame selection in this sandbox.
  // WebKit also emits a pageerror for this deliberate denied-access probe.
  const deniedProbe = page.waitForEvent("pageerror", {
    predicate: (error) => error.message.startsWith("Blocked a frame at") && error.message.includes("lacks the \"allow-same-origin\" flag"),
    timeout: 1000,
  }).catch(() => null);
  assert.equal(await iframe.evaluate((element) => element.contentDocument), null);
  const expectedError = await deniedProbe;
  if (expectedError) errors.splice(errors.indexOf(expectedError.message), 1);
  const frameBox = await iframe.boundingBox();
  const passageRect = await frame.locator("p").evaluate((element) => {
    const range = document.createRange(); range.selectNodeContents(element);
    const rect = range.getBoundingClientRect();
    return { left: rect.left, right: rect.right, y: (rect.top + rect.bottom) / 2 };
  });
  await page.mouse.move(frameBox.x + passageRect.left - 1, frameBox.y + passageRect.y);
  await page.mouse.down();
  await page.mouse.move(frameBox.x + passageRect.right + 1, frameBox.y + passageRect.y, { steps: 10 });
  await page.mouse.up();
  await feedback.waitFor();
  assert.equal(await page.evaluate(() => document.activeElement.tagName), "IFRAME");
  await page.getByRole("button", { name: "Humanize", exact: true }).click();
  await page.getByRole("button", { name: "Use in reply", exact: true }).waitFor();
  if (process.env.CXMAIL_SELECTION_SCREENSHOT) await page.screenshot({ path: process.env.CXMAIL_SELECTION_SCREENSHOT });
  assert.equal(await frame.locator("p").textContent(), "Incoming email passage for feedback.");
  await page.getByRole("button", { name: "Use in reply", exact: true }).click();
  const replies = JSON.parse(await page.getByTestId("replies").textContent());
  assert.equal(replies[0].defaultBody, "<p>Natural version</p>");
  assert.deepEqual(replies[0].replyContext, { accountId: "email-account", folder: "Archive", uid: 42 });
  assert.equal(replies[0].inReplyTo, "<email@example.test>");

  await selectMail();
  const input = page.getByRole("textbox", { name: "Tell Claude what to change" });
  await input.fill("Explain this passage"); await input.press("Enter");
  await page.waitForFunction(() => document.querySelector('[data-testid="chat"]')?.textContent.includes("Explain this passage"));
  const chat = JSON.parse(await page.getByTestId("chat").textContent());
  assert.match(chat[0].text, /> Incoming email passage for feedback\./);

  await selectMail();
  // Native text context menus remain unclaimed by the frame script.
  assert.equal(await frame.locator("p").evaluate((element) => element.dispatchEvent(
    new MouseEvent("contextmenu", { bubbles: true, cancelable: true, button: 2 }),
  )), true);
  await frame.locator("p").press("Escape");
  await feedback.waitFor({ state: "detached" });
  await page.waitForTimeout(100);
  assert.equal(await feedback.count(), 0, "Escape must not reopen feedback on keyup");
  await selectMail();
  await frame.locator("p").evaluate(() => getSelection().removeAllRanges());
  await feedback.waitFor({ state: "detached" });

  await page.getByTestId("plain").locator("pre").evaluate((element) => {
    const range = document.createRange(); range.selectNodeContents(element);
    const selection = getSelection(); selection.removeAllRanges(); selection.addRange(range);
    element.dispatchEvent(new MouseEvent("mouseup", { bubbles: true }));
  });
  await feedback.waitFor();
  await page.getByRole("textbox", { name: "Tell Claude what to change" }).fill("Explain plain mail");
  await page.getByRole("textbox", { name: "Tell Claude what to change" }).press("Enter");
  await page.waitForFunction(() => document.querySelector('[data-testid="chat"]')?.textContent.includes("Explain plain mail"));
  assert.equal(errors.length, 0, errors.join("\n"));
  console.log("WebKit passed: immediate feedback without losing selection/focus, draft replacement/undo, protected blocks, stale-result fallback, read-only reply/chat, native contextmenu, Escape and selection clearing.");
} finally {
  await browser.close();
}
