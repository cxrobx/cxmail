// Script injected into the sandboxed email iframe (see EmailFrame.tsx).
// Runs in a sandboxed context (allow-scripts + allow-top-navigation-by-user-activation,
// no allow-same-origin). ammonia strips all <script> tags from email HTML, so this
// is the only script that will execute. Link clicks navigate the top frame via
// <base target="_top"> and are caught by Tauri's on_navigation handler.
//
// WHY THIS IS AN EXTERNAL FILE AND NOT INLINE (gotcha #29):
// A srcdoc iframe inherits the embedder's Content-Security-Policy. The app CSP
// (tauri.conf.json -> app.security.csp) ends in `script-src 'self'`, which blocks
// inline scripts here — silently, since a production build ships no DevTools. With
// this script blocked, no cxmail-frame-height message is ever posted and every email
// body renders clipped to EmailFrame's initial 200px height. Loading from '/'
// satisfies `'self'`. A 'sha256-...' hash allowlist is NOT a valid alternative:
// verified that WebKit (WKWebView) ignores hash allowances for inline scripts in
// sandboxed srcdoc frames even though Chromium honors them.
(function () {
  function reportHeight() {
    var h = Math.max(
      document.body ? document.body.scrollHeight : 0,
      document.body ? document.body.offsetHeight : 0,
      document.documentElement ? document.documentElement.scrollHeight : 0,
      document.documentElement ? document.documentElement.offsetHeight : 0,
      200
    );
    parent.postMessage({ type: 'cxmail-frame-height', height: h }, '*');
  }

  // Report height on load, after images settle, and on DOM mutations
  reportHeight();
  setTimeout(reportHeight, 50);
  setTimeout(reportHeight, 300);
  if (window.MutationObserver && document.body) {
    new MutationObserver(reportHeight).observe(document.body, {
      childList: true, subtree: true, characterData: true, attributes: true
    });
  }

  // Right-click on an <img>: suppress WKWebView's broken native menu (its
  // Save/Copy/Open items need a WKUIDelegate Tauri doesn't wire) and ask the
  // parent to render a CXMail-styled menu. Non-image right-clicks fall through
  // so native text-selection menus (Copy, etc.) keep working.
  document.addEventListener('contextmenu', function (e) {
    var target = e.target;
    var img = target && target.closest ? target.closest('img') : null;
    if (!img) return;
    var src = img.currentSrc || img.getAttribute('src') || '';
    if (!src) return; // defense: blocked remote images have empty src
    e.preventDefault();
    parent.postMessage({
      type: 'cxmail-image-contextmenu',
      src: src,
      clientX: e.clientX,
      clientY: e.clientY,
    }, '*');
  });

  // Tell the parent to dismiss its image menu when the user clicks or scrolls
  // inside the iframe — the parent's outside-pointerdown listener never sees
  // events that originate inside the iframe.
  document.addEventListener('pointerdown', function () {
    parent.postMessage({ type: 'cxmail-iframe-pointerdown' }, '*');
  }, true);
  document.addEventListener('wheel', function () {
    parent.postMessage({ type: 'cxmail-iframe-wheel' }, '*');
  }, { passive: true });

  // Intercept link clicks and relay to parent via postMessage
  document.addEventListener('click', function (e) {
    var target = e.target;
    var anchor = target.closest ? target.closest('a') : null;
    // If click was on a wrapper (td, div) that contains a single link, use it
    if (!anchor && target.querySelector) {
      anchor = target.querySelector('a');
    }
    if (!anchor) return;
    var href = anchor.getAttribute('href');
    if (!href) return;
    if (href.indexOf('http://') === 0 || href.indexOf('https://') === 0 || href.indexOf('mailto:') === 0) {
      e.preventDefault();
      e.stopPropagation();
      parent.postMessage({ type: 'cxmail-open-link', href: href }, '*');
    }
  });

  // Restore remote image src tags when parent grants permission. Sandboxed
  // srcDoc iframes have a null/'' origin, so we filter on event.source rather
  // than event.origin. ammonia URL-allowlisted the original src at parse time
  // (http/https only), so this only restores values the sanitizer accepted.
  window.addEventListener('message', function (e) {
    if (e.source !== window.parent) return;
    var data = e.data;
    if (!data || data.type !== 'cxmail-load-images') return;
    var imgs = document.querySelectorAll('img[data-original-src]');
    for (var i = 0; i < imgs.length; i++) {
      var img = imgs[i];
      var orig = img.getAttribute('data-original-src');
      if (orig) {
        img.setAttribute('src', orig);
        img.removeAttribute('data-original-src');
      }
    }
    reportHeight();
  });
})();
