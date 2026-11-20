import { n as createWorkerClient } from "/assets/worker-rpc-CoD-MTl9.js";

const kind = new URLSearchParams(location.search).get("kind") ?? "word";
const workers = {
  word: "document.worker-BJ3XBWBI.js",
  xlsx: "spreadsheet.worker-AXWWY4sf.js",
  pptx: "presentation.worker-o5xvygYW.js",
};
const engine = createWorkerClient(new Worker(`/assets/${workers[kind]}`, { type: "module" }));
const style = document.createElement("style");
document.head.append(style);
let pixels = 0, pagesPrinted = 0;

function checkPage(width, height) {
  pixels += width * height;
  if (!(width > 0 && width <= 1800 && height > 0 && height <= 2400) || pixels > 24e6)
    throw new Error("Page size limit");
}

function appendPage(bitmap, width, height, title) {
  if (!bitmap) throw new Error("Missing page");
  const canvas = document.createElement("canvas");
  canvas.width = bitmap.width;
  canvas.height = bitmap.height;
  canvas.style.width = `${width}px`;
  canvas.style.height = `${height}px`;
  canvas.getContext("2d").drawImage(bitmap, 0, 0);
  bitmap.close();
  const sheet = document.createElement("section");
  const name = `preview${pagesPrinted++}`;
  sheet.style.page = name;
  let labelHeight = 0;
  if (title) {
    const heading = document.createElement("header");
    heading.textContent = title;
    heading.style.cssText = "height:32px;line-height:32px;padding:0 8px;font:600 16px/32px system-ui;white-space:nowrap;overflow:hidden";
    sheet.append(heading);
    labelHeight = 32;
  }
  style.sheet.insertRule(`@page ${name} { size: ${width}px ${height + labelHeight}px; margin: 0 }`);
  sheet.append(canvas);
  document.body.append(sheet);
}

function checkLength(items) {
  if (!items.length || items.length > 24) throw new Error("Page limit");
}

async function word(opened) {
  let { pages, loading } = opened;
  while (loading) {
    const step = await engine.call("layout");
    if (step.done) ({ pages, loading } = { pages: step.pages, loading: false });
  }
  checkLength(pages);
  for (const [index, { width, height }] of pages.entries()) {
    checkPage(width, height);
    appendPage(await engine.call("paint", { index, scale: 1, ratio: 2 }), width, height);
  }
}

async function slides(pages) {
  checkLength(pages);
  for (const [index, { width, height }] of pages.entries()) {
    checkPage(width, height);
    const result = await engine.call("paint", { index, density: 2 });
    if (result.kind !== "painted" || result.vectors.length)
      throw new Error("Unsupported slide content");
    appendPage(result.bitmap, width, height);
  }
}

async function workbook({ tabs, layout: first }) {
  checkLength(tabs);
  for (const { index: sheet, name } of tabs) {
    const layout = first.info.activeSheet === sheet && pagesPrinted === 0
      ? first
      : await engine.call("show", { sheet });
    const end = layout.used && await engine.call("position", {
      sheet, cell: { row: layout.used.bottom, col: layout.used.right },
    });
    if (layout.used && !end) throw new Error("Sheet range unavailable");
    const width = Math.ceil(Math.max(layout.info.contentWidth, layout.chartWidth, end?.next.x ?? 0, 360));
    const height = Math.ceil(Math.max(layout.info.contentHeight, layout.chartHeight, end?.next.y ?? 0, 80));
    checkPage(width, height + 32);
    const frame = await engine.call("frame", { sheet, x: 0, y: 0, width, height, ratio: 2, offset: {x: 0, y: 0} });
    appendPage(frame?.bitmap, width, height, name);
  }
}

try {
  const response = await fetch("/document.docx");
  if (!response.ok) throw new Error("Document unavailable");
  const bytes = await response.arrayBuffer();
  const opened = await engine.call("open", { bytes }, { transfer: [bytes] });
  await ({ word, pptx: slides, xlsx: workbook })[kind](opened);
  document.body.dataset.result = "done";
} catch {
  document.body.dataset.result = "failed";
} finally {
  engine.close();
}
