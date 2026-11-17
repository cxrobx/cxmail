import test from "node:test";
import assert from "node:assert/strict";
import worker from "../index.js";
const id = "0278e6ec-4dd4-4661-bd7e-b1cf34edb637";
const env = new Proxy({ LINK_APP_ID: "ABCDEFGHIJ.app.cxmail.remote" }, {
  get(target, key) {
    if (key === "LINK_APP_ID") return target[key];
    throw new Error(`Public metadata accessed an unexpected binding: ${String(key)}`);
  }
});
const get = (path, method = "GET", config = env) => worker.fetch(new Request(`https://links.example.com${path}`, { method }), config);
test("association advertises only draft links", async () => {
  const r = await get("/.well-known/apple-app-site-association");
  assert.equal(r.status, 200);
  assert.deepEqual(await r.json(), { applinks: { details: [{ appIDs: [env.LINK_APP_ID], components: [{ "/": "/draft/*" }] }] } });
});
test("all UUIDs get the generic fallback; no existence oracle or network access", async () => {
  for (const draft of [id, "00000000-0000-0000-0000-000000000000"]) {
    const r = await get(`/draft/${draft}`);
    assert.equal(r.status, 200);
    const body = await r.text();
    assert.ok(body.includes(`cxmail://draft/${draft}`));
    assert.ok(!body.includes("<img") && !body.includes("https://"));
    assert.equal(r.headers.get("Cache-Control"), "no-store");
    assert.equal(r.headers.get("Referrer-Policy"), "no-referrer");
    assert.ok(r.headers.get("Content-Security-Policy").includes("default-src 'none'"));
  }
});
test("strict paths, methods and configuration", async () => {
  for (const path of ["/", "/api/drafts", `/draft/${id}/`, `/draft/${id}?token=x`, "/draft/<script>", `/draft/${id.toUpperCase()}`])
    assert.equal((await get(path)).status, 404, path);
  assert.equal((await get(`/draft/${id}`, "POST")).status, 404);
  assert.equal((await get(`/draft/${id}`, "GET", {})).status, 404);
  const head = await get(`/draft/${id}`, "HEAD");
  assert.equal(head.status, 200); assert.equal(await head.text(), "");
});
