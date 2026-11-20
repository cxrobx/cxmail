const fs = require("node:fs");
const path = require("node:path");
const os = require("node:os");
const { chromium } = require("playwright-core");

const origin = "http://cxmail-word-preview.invalid";
const installed = path.join(os.homedir(), ".t3/runtime/versions/0.1.7/client/assets");
const kind = process.argv[2] ?? "word";
const workers = {
  word: "document.worker-BJ3XBWBI.js",
  xlsx: "spreadsheet.worker-AXWWY4sf.js",
  pptx: "presentation.worker-o5xvygYW.js",
};
const assetBase = path.join(os.homedir(), "Library/Application Support/com.cxmail.remote/word-preview");
const assetRoots = [path.join(assetBase, "0.1.7-office/assets"), path.join(assetBase, "0.1.7/assets"), installed];
const assets = workers[kind] && assetRoots.find((root) => fs.existsSync(path.join(root, workers[kind])));
const mime = { ".js": "text/javascript", ".ttf": "font/ttf", ".woff2": "font/woff2", ".wasm": "application/wasm" };

async function render() {
  if (!assets) throw new Error("Office renderer unavailable");
  const source = fs.readFileSync(0);
  const browser = await chromium.launch({
    executablePath: "/Applications/Google Chrome.app/Contents/MacOS/Google Chrome",
    chromiumSandbox: true,
    timeout: 10000,
  });
  try {
    const context = await browser.newContext({ serviceWorkers: "block" });
    await context.route("**/*", async (route) => {
      const url = new URL(route.request().url());
      if (url.origin !== origin || route.request().method() !== "GET") return route.abort();
      if (url.pathname === "/") return route.fulfill({ path: path.join(__dirname, "index.html"), contentType: "text/html" });
      if (url.pathname === "/preview.js") return route.fulfill({ path: path.join(__dirname, "preview.js"), contentType: "text/javascript" });
      if (url.pathname === "/document.docx") return route.fulfill({ body: source, contentType: "application/octet-stream" });
      const asset = /^\/assets\/([\w.-]+)$/.exec(url.pathname)?.[1];
      const contentType = asset && mime[path.extname(asset)];
      if (!contentType) return route.abort();
      const file = path.join(assets, asset);
      if (!fs.existsSync(file)) return route.abort();
      return route.fulfill({ path: file, contentType });
    });
    const page = await context.newPage();
    await page.goto(`${origin}/?kind=${kind}`, { timeout: 12000 });
    await page.waitForFunction(() => document.body.dataset.result, { timeout: 20000 });
    if (await page.locator("body").getAttribute("data-result") !== "done") throw new Error("Word layout failed.");
    const pdf = await page.pdf({ preferCSSPageSize: true, printBackground: true });
    process.stdout.write(pdf);
  } finally {
    await browser.close();
  }
}

render().catch(() => { process.exitCode = 1; });
