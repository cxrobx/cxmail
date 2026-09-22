import { useEffect, useMemo, useRef, useState } from "react";
import { createPortal } from "react-dom";
import { open } from "@tauri-apps/plugin-shell";
import { save } from "@tauri-apps/plugin-dialog";
import { writeText } from "@tauri-apps/plugin-clipboard-manager";
import { ImageOff } from "lucide-react";
import { useResolvedTheme } from "../../hooks/useResolvedTheme";
import { api } from "../../lib/tauri";

interface EmailFrameProps {
  html: string;
  /** Sender address used for the per-sender image-trust allowlist. When
   * omitted, blocked-image banner is hidden (no sender to attribute trust to). */
  senderEmail?: string;
}

// Script injected into the iframe. Runs in a sandboxed context (allow-scripts +
// allow-top-navigation-by-user-activation, no allow-same-origin). ammonia strips
// all <script> tags from email HTML, so this is the only script that will execute.
// Link clicks navigate the top frame via <base target="_top"> and are caught by
// Tauri's on_navigation handler.
//
// It MUST stay an external file rather than an inline <script> (gotcha #29): a
// srcdoc iframe inherits the embedder's CSP, and the app CSP ends in
// `script-src 'self'`, which blocks inline scripts here — silently, since release
// builds ship no DevTools. Blocked, nothing ever posts cxmail-frame-height and
// every email body renders clipped to the initial 200px frameHeight below.
// The absolute path resolves against the parent document (srcdoc inherits its
// base URL), i.e. the app origin, which is what `'self'` permits.
// Source lives at public/email-frame.js — Vite copies public/ to the dist root
// unhashed, so this URL is stable in dev and in a bundled app.
const FRAME_SCRIPT = `<script src="/email-frame.js"></script>`;

function buildFrameDocument(html: string, isDark: boolean): string {
  return `<!doctype html>
<html>
  <head>
    <meta charset="utf-8" />
    <base target="_top" />
    <style>
      html, body {
        margin: 0;
        padding: 0;
        background: transparent;
        font-family: -apple-system, BlinkMacSystemFont, sans-serif;
        font-size: 14px;
        line-height: 1.5;
        color: ${isDark ? "rgb(232, 220, 200)" : "rgb(30, 25, 15)"};
        word-wrap: break-word;
      }
      body {
        padding: 0 12px;
      }
      a { cursor: pointer; }
      a:not([style*="background"]) { color: rgb(10, 132, 255); }
      img { max-width: 100%; height: auto; }
      img[data-original-src] { display: none !important; }
      blockquote {
        border-left: 3px solid ${isDark ? "rgb(74, 68, 57)" : "rgb(200, 195, 185)"};
        margin: 8px 0;
        padding-left: 12px;
        color: ${isDark ? "rgb(160, 144, 120)" : "rgb(100, 90, 75)"};
      }
      hr { border-color: ${isDark ? "rgb(74, 68, 57)" : "rgb(220, 215, 205)"}; }
      ${isDark ? `
      div:not([style*="background"]),
      p,
      span:not([style*="background"]),
      td:not([style*="background"]),
      th,
      li,
      h1,
      h2,
      h3,
      h4,
      h5,
      h6,
      font {
        color: rgb(232, 220, 200) !important;
      }
      a:not([style*="background"]) {
        color: rgb(10, 132, 255) !important;
      }
      table:not([style*="background"]),
      tr:not([style*="background"]),
      td:not([style*="background"]),
      th:not([style*="background"]),
      div:not([style*="background"]) {
        background-color: transparent !important;
      }
      table, tr, td, th {
        border-color: rgb(74, 68, 57) !important;
      }
      ` : ""}
    </style>
  </head>
  <body>
    <div class="email-body">${html}</div>
    ${FRAME_SCRIPT}
  </body>
</html>`;
}

export default function EmailFrame({ html, senderEmail }: EmailFrameProps) {
  const iframeRef = useRef<HTMLIFrameElement>(null);
  const theme = useResolvedTheme();
  const [frameHeight, setFrameHeight] = useState(200);
  // Tri-state on purpose: 'pending' suppresses the banner during the trust
  // query so a trusted sender doesn't briefly flash a "blocked" prompt before
  // we postMessage the iframe to load.
  const [trustState, setTrustState] = useState<"pending" | "trusted" | "untrusted">(
    senderEmail ? "pending" : "untrusted",
  );
  const [imagesShown, setImagesShown] = useState(false);
  // Image right-click menu state. Coordinates are viewport-relative so we can
  // use position: fixed in the portal — the reading-pane scroll container
  // would otherwise shift an absolutely-positioned menu off the cursor.
  const [imageMenu, setImageMenu] = useState<
    | { src: string; x: number; y: number; isHttps: boolean }
    | null
  >(null);
  const isDark = theme === "dark";
  const srcDoc = useMemo(() => buildFrameDocument(html, isDark), [html, isDark]);
  const hasBlockedImages = useMemo(() => /data-original-src=/i.test(html), [html]);

  // Resolve trust state for this sender on mount / when sender changes.
  useEffect(() => {
    if (!senderEmail) {
      setTrustState("untrusted");
      return;
    }
    let cancelled = false;
    setTrustState("pending");
    api.imageTrust
      .isTrusted(senderEmail)
      .then((trusted) => {
        if (cancelled) return;
        setTrustState(trusted ? "trusted" : "untrusted");
      })
      .catch(() => {
        if (!cancelled) setTrustState("untrusted");
      });
    return () => {
      cancelled = true;
    };
  }, [senderEmail]);

  // Reset per-message state when the html itself changes (e.g., switching
  // to a different message). Without this, "imagesShown=true" would stick
  // across messages even though the new message hasn't been unblocked.
  useEffect(() => {
    setImagesShown(false);
  }, [html]);

  function postLoadImages() {
    const win = iframeRef.current?.contentWindow;
    if (!win) return;
    win.postMessage({ type: "cxmail-load-images" }, "*");
  }

  function handleShowImagesOnce() {
    setImagesShown(true);
    postLoadImages();
  }

  async function handleAlwaysTrust() {
    if (!senderEmail) return;
    try {
      await api.imageTrust.trust(senderEmail);
      setTrustState("trusted");
      setImagesShown(true);
      postLoadImages();
    } catch (err) {
      console.error("Failed to trust sender", err);
    }
  }

  // Auto-load on iframe ready when sender is trusted. Re-runs on srcDoc
  // change (new message, theme flip — iframe reloads) so the trusted state
  // is re-applied after each reload.
  function handleIframeLoad() {
    if (trustState === "trusted") {
      postLoadImages();
      setImagesShown(true);
    }
  }
  // Cover the race where trust resolves AFTER the iframe finished loading.
  useEffect(() => {
    if (trustState === "trusted" && !imagesShown) {
      postLoadImages();
      setImagesShown(true);
    }
  }, [trustState, imagesShown]);

  useEffect(() => {
    const handleMessage = (event: MessageEvent) => {
      // Only accept messages from our iframe
      if (event.source !== iframeRef.current?.contentWindow) return;

      const data = event.data;
      if (!data || typeof data !== "object") return;

      if (data.type === "cxmail-open-link" && typeof data.href === "string") {
        // mailto: links open CXMail's own composer (via the AppLayout funnel),
        // not the OS handler — otherwise, now that CXMail is the default mailto
        // handler, open() would bounce the click right back to us.
        if (/^mailto:/i.test(data.href)) {
          window.dispatchEvent(new CustomEvent("cxmail:mailto", { detail: data.href }));
        } else {
          void open(data.href).catch(console.error);
        }
      } else if (data.type === "cxmail-frame-height" && typeof data.height === "number") {
        setFrameHeight(Math.max(data.height, 200));
      } else if (data.type === "cxmail-image-contextmenu" && typeof data.src === "string") {
        const src: string = data.src;
        // Defense in depth: only data:image/ and https:// sources get a menu.
        const isData = src.startsWith("data:image/");
        const isHttps = src.startsWith("https://");
        if (!isData && !isHttps) return;
        const rect = iframeRef.current?.getBoundingClientRect();
        const localX = typeof data.clientX === "number" ? data.clientX : 0;
        const localY = typeof data.clientY === "number" ? data.clientY : 0;
        const viewportX = (rect?.left ?? 0) + localX;
        const viewportY = (rect?.top ?? 0) + localY;
        // Clamp to viewport so a click near the edge doesn't push the menu
        // off-screen. Approximate menu size; final position is corrected if
        // needed below by setting menu max-width/height.
        const clampedX = Math.min(Math.max(viewportX, 8), window.innerWidth - 220);
        const clampedY = Math.min(Math.max(viewportY, 8), window.innerHeight - 200);
        setImageMenu({ src, x: clampedX, y: clampedY, isHttps });
      } else if (
        data.type === "cxmail-iframe-pointerdown" ||
        data.type === "cxmail-iframe-wheel"
      ) {
        setImageMenu(null);
      }
    };

    window.addEventListener("message", handleMessage);
    return () => window.removeEventListener("message", handleMessage);
  }, []);

  // Close the image menu on outside parent-side clicks, Escape, or window blur.
  // (Clicks INSIDE the iframe arrive via the iframe-pointerdown postMessage
  // bridge above — parent listeners never see those events.)
  useEffect(() => {
    if (!imageMenu) return;
    const close = () => setImageMenu(null);
    const onKey = (e: KeyboardEvent) => {
      if (e.key === "Escape") close();
    };
    window.addEventListener("pointerdown", close);
    window.addEventListener("blur", close);
    window.addEventListener("keydown", onKey);
    return () => {
      window.removeEventListener("pointerdown", close);
      window.removeEventListener("blur", close);
      window.removeEventListener("keydown", onKey);
    };
  }, [imageMenu]);

  function suggestedFilename(src: string): string {
    if (src.startsWith("data:image/")) {
      const ext = src.slice("data:image/".length).split(/[;,]/, 1)[0] || "png";
      return `image.${ext === "jpeg" ? "jpg" : ext}`;
    }
    try {
      const u = new URL(src);
      const last = u.pathname.split("/").pop() || "image";
      return last.includes(".") ? last : `${last}.png`;
    } catch {
      return "image.png";
    }
  }

  async function handleSaveImage(src: string) {
    setImageMenu(null);
    try {
      const dest = await save({ defaultPath: suggestedFilename(src) });
      if (!dest) return;
      await api.images.saveToPath(src, dest);
    } catch (err) {
      console.error("Save image failed", err);
    }
  }

  async function handleCopyImage(src: string) {
    setImageMenu(null);
    try {
      await api.images.copyToClipboard(src);
    } catch (err) {
      console.error("Copy image failed", err);
    }
  }

  async function handleCopyImageAddress(src: string) {
    setImageMenu(null);
    try {
      await writeText(src);
    } catch (err) {
      console.error("Copy image address failed", err);
    }
  }

  async function handleOpenInBrowser(src: string) {
    setImageMenu(null);
    try {
      await open(src);
    } catch (err) {
      console.error("Open image failed", err);
    }
  }

  const showBanner =
    !!senderEmail &&
    hasBlockedImages &&
    !imagesShown &&
    trustState === "untrusted";

  return (
    <div className="w-full">
      {showBanner && (
        <div
          className="mx-3 mb-2 flex flex-wrap items-center gap-x-3 gap-y-1 rounded-md border border-border-subtle bg-elevated px-3 py-2 text-xs text-content-secondary"
          role="status"
        >
          <ImageOff className="h-3.5 w-3.5 shrink-0 text-content-muted" />
          <span className="min-w-0 flex-1 truncate">
            Images blocked from{" "}
            <span className="font-medium text-content">{senderEmail}</span>
          </span>
          <div className="flex shrink-0 items-center gap-2">
            <button
              type="button"
              onClick={handleShowImagesOnce}
              className="rounded bg-surface px-2 py-0.5 text-xs text-content hover:bg-elevated"
            >
              Show images
            </button>
            <button
              type="button"
              onClick={handleAlwaysTrust}
              className="rounded px-2 py-0.5 text-xs text-content-muted hover:text-content"
            >
              Always trust this sender
            </button>
          </div>
        </div>
      )}
      <iframe
        ref={iframeRef}
        title="Email content"
        sandbox="allow-scripts allow-top-navigation-by-user-activation"
        srcDoc={srcDoc}
        onLoad={handleIframeLoad}
        className="w-full border-0"
        style={{ height: frameHeight }}
      />
      {imageMenu &&
        createPortal(
          <div
            role="menu"
            onPointerDown={(e) => e.stopPropagation()}
            className="fixed z-50 min-w-[200px] rounded-md border border-border-subtle bg-elevated py-1 text-sm text-content shadow-lg"
            style={{ left: imageMenu.x, top: imageMenu.y }}
          >
            <button
              type="button"
              role="menuitem"
              onClick={() => handleSaveImage(imageMenu.src)}
              className="block w-full px-3 py-1.5 text-left hover:bg-surface"
            >
              Save Image As…
            </button>
            <button
              type="button"
              role="menuitem"
              onClick={() => handleCopyImage(imageMenu.src)}
              className="block w-full px-3 py-1.5 text-left hover:bg-surface"
            >
              Copy Image
            </button>
            {imageMenu.isHttps && (
              <>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => handleCopyImageAddress(imageMenu.src)}
                  className="block w-full px-3 py-1.5 text-left hover:bg-surface"
                >
                  Copy Image Address
                </button>
                <button
                  type="button"
                  role="menuitem"
                  onClick={() => handleOpenInBrowser(imageMenu.src)}
                  className="block w-full px-3 py-1.5 text-left hover:bg-surface"
                >
                  Open in Browser
                </button>
              </>
            )}
          </div>,
          document.body,
        )}
    </div>
  );
}
