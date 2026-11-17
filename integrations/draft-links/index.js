// Public navigation metadata only. No mail API, storage, credentials or fetches.
const ID = "[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}";
const DRAFT = new RegExp(`^/draft/(${ID})$`);
export default {
  async fetch(request, env) {
    const headers = { "Cache-Control": "no-store", "Referrer-Policy": "no-referrer",
      "X-Content-Type-Options": "nosniff", "X-Robots-Tag": "noindex, nofollow",
      "Content-Security-Policy": "default-src 'none'; script-src 'unsafe-inline'; base-uri 'none'; frame-ancestors 'none'; form-action 'none'" };
    const reply = (body, status, type) => new Response(request.method === "HEAD" ? null : body,
      { status, headers: { ...headers, "Content-Type": type } });
    const url = new URL(request.url);
    if (!["GET", "HEAD"].includes(request.method) || url.search ||
        !/^[A-Z0-9]{10}\.[A-Za-z0-9.-]+$/.test(env.LINK_APP_ID ?? ""))
      return reply("Not found", 404, "text/plain");
    if (url.pathname === "/.well-known/apple-app-site-association")
      return reply(JSON.stringify({ applinks: { details: [{ appIDs: [env.LINK_APP_ID],
        components: [{ "/": "/draft/*" }] }] } }), 200, "application/json");
    const match = DRAFT.exec(url.pathname);
    if (!match) return reply("Not found", 404, "text/plain");
    const target = `cxmail://draft/${match[1]}`;
    return reply(`<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><meta name="referrer" content="no-referrer"><title>Open in CXMail</title><body><h1>Open in CXMail</h1><p>Review this draft in CXMail on your iPhone.</p><p><a href="${target}">Open in CXMail</a></p><p>CXMail must be installed and connected to your Mac through Tailscale.</p><script>location.replace(${JSON.stringify(target)})</script></body></html>`, 200, "text/html; charset=utf-8");
  }
};
