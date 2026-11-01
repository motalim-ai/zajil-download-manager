// Zajil's in-app browser (A6, layer 3): the page's videos, for Zajil only.
// Run before the page's own scripts, in every frame. From the desktop
// extension's video button (apps/extension/src/video-button.js): the same
// size rule and the same refusal of protected (EME) videos; no button, no
// reading ahead: nothing is fetched until the user taps «Download».
//
// It reports only to Zajil, through the object Zajil gives (a
// WebMessageListener), which it takes before the page can run, with this
// load's token, which the page never sees. Zajil treats every report as
// untrusted page data all the same. The badge «found by Zajil» is a
// closed shadow root that takes no clicks.
(() => {
  "use strict";
  const port = globalThis.__ZAJIL_BRIDGE__;
  if (!port || typeof port.postMessage !== "function") return;
  try {
    delete globalThis.__ZAJIL_BRIDGE__;
  } catch {}
  const TOKEN = __ZAJIL_TOKEN__;
  const LABEL = __ZAJIL_LABEL__;
  // The page's own scripts run later: what this script uses is kept now.
  const post = port.postMessage.bind(port);
  const stringify = JSON.stringify;
  const all = Document.prototype.querySelectorAll;
  const rectOf = Element.prototype.getBoundingClientRect;
  const later = setTimeout;

  /** The smallest video that counts (not an icon or a preview), as the extension's button. */
  const MIN_WIDTH = 200;
  const MIN_HEIGHT = 120;

  /** Videos that fired `encrypted`: played through EME (ADR-0009, section 6). */
  const encrypted = new WeakSet();
  document.addEventListener(
    "encrypted",
    (event) => {
      if (event.target instanceof HTMLVideoElement) encrypted.add(event.target);
    },
    true,
  );

  const videos = () => {
    const found = [];
    for (const video of all.call(document, "video")) {
      const box = rectOf.call(video);
      if (box.width < MIN_WIDTH || box.height < MIN_HEIGHT) continue;
      let src = video.currentSrc || video.src || "";
      if (!src) {
        const source = video.querySelector("source[src]");
        src = source ? source.src : "";
      }
      found.push({
        video,
        src: String(src).slice(0, 4096),
        w: Math.round(box.width),
        h: Math.round(box.height),
        d: Number.isFinite(video.duration) ? Math.round(video.duration) : 0,
        p: video.mediaKeys != null || encrypted.has(video),
      });
      if (found.length >= 20) break;
    }
    return found;
  };

  // The badge over each video found, in a closed shadow root.
  let host = null;
  let shadow = null;
  const badges = () => {
    if (!host) {
      host = document.createElement("zajil-found");
      shadow = host.attachShadow({ mode: "closed" });
      (document.documentElement || document).appendChild(host);
    }
    return shadow;
  };
  const place = (list) => {
    if (!list.length && !host) return;
    const root = badges();
    root.replaceChildren();
    const style = document.createElement("style");
    style.textContent =
      ":host{all:initial!important;position:fixed!important;inset:0!important;pointer-events:none!important;z-index:2147483647!important}" +
      ".b{position:fixed;display:flex;align-items:center;gap:4px;height:24px;padding:0 8px;border-radius:999px;" +
      "background:rgba(28,108,208,.92);color:#fff;font:600 12px system-ui,sans-serif;pointer-events:none;direction:rtl}";
    root.appendChild(style);
    for (const one of list) {
      if (one.p) continue;
      const box = rectOf.call(one.video);
      const badge = document.createElement("span");
      badge.className = "b";
      badge.textContent = "↓ " + LABEL;
      badge.style.top = Math.max(0, box.top + 8) + "px";
      badge.style.left = Math.max(0, box.left + 8) + "px";
      root.appendChild(badge);
    }
  };

  let last = "";
  let current = [];
  const report = () => {
    current = videos();
    const message = stringify({
      t: TOKEN,
      title: String(document.title || "").slice(0, 200),
      videos: current.map(({ src, w, h, d, p }) => ({ src, w, h, d, p })),
    });
    place(current);
    if (message === last) return;
    last = message;
    try {
      post(message);
    } catch {}
  };

  let timer = 0;
  const soon = () => {
    clearTimeout(timer);
    timer = later(report, 500);
  };
  const watch = () => {
    new MutationObserver(soon).observe(document, { childList: true, subtree: true, attributes: true, attributeFilter: ["src"] });
    for (const type of ["loadedmetadata", "play", "emptied"]) document.addEventListener(type, soon, true);
    addEventListener("scroll", () => place(current), { passive: true, capture: true });
    addEventListener("resize", soon, { passive: true });
    soon();
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", watch, { once: true });
  else watch();
})();
