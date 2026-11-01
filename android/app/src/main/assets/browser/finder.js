// Zajil's in-app browser (A6, layer 3): the page's videos, for Zajil only.
// Run before the page's own scripts, in every frame. From the desktop
// extension's video button (apps/extension/src/video-button.js): the same
// size rule and the same refusal of protected (EME) videos; no button, no
// reading ahead: nothing is fetched until the user taps «Download».
//
// It reports only to Zajil, through the object Zajil gives (a
// WebMessageListener), which it takes before the page can run, with this
// load's token, which the page never sees. Zajil treats every report as
// untrusted page data all the same. Nothing is drawn over the page: the
// app's own «Download» button says what was found (the owner's request).
(() => {
  "use strict";
  const port = globalThis.__ZAJIL_BRIDGE__;
  if (!port || typeof port.postMessage !== "function") return;
  try {
    delete globalThis.__ZAJIL_BRIDGE__;
  } catch {}
  const TOKEN = __ZAJIL_TOKEN__;
  // The page's own scripts run later: what this script uses is kept now.
  const post = port.postMessage.bind(port);
  const stringify = JSON.stringify;
  const all = Document.prototype.querySelectorAll;
  const inside = Element.prototype.querySelectorAll;
  const attribute = Element.prototype.getAttribute;
  const rectOf = Element.prototype.getBoundingClientRect;
  const later = setTimeout;
  const keysOf = Object.keys;
  const Address = URL;

  /**
   * A video's own page on a feed whose address is not it (Instagram's,
   * Facebook's, X's): the nearest link to a video page around the video,
   * never past an element that holds another video (that is the feed).
   * Zajil checks it again: a video page of the page's own site, or nothing.
   */
  const PAGE =
    /^https:\/\/(?:www\.|m\.|web\.|mobile\.)?(?:instagram\.com\/(?:p|reel|reels|tv)\/[\w-]+|facebook\.com\/(?:[^?#]+\/videos\/\d+|watch\/?\?v=\d+|reel\/\d+)|tiktok\.com\/@[\w.-]+\/video\/\d+|(?:x|twitter)\.com\/\w+\/status\/\d+|snapchat\.com\/(?:@[\w.-]+\/)?spotlight\/[\w-]+)/;
  const pageOf = (href) => {
    try {
      // Instagram's «/user/reel/ID» is its «/reel/ID».
      const url = new Address(href, location.href).href.replace(/(instagram\.com)\/[\w.]+\/(p|reel|reels|tv)\//, "$1/$2/");
      const match = PAGE.exec(url);
      return match ? match[0] : "";
    } catch {
      return "";
    }
  };
  const linkedPage = (video) => {
    let node = video.parentElement;
    for (let depth = 0; node && depth < 12; depth++, node = node.parentElement) {
      if (depth > 0 && inside.call(node, "video").length > 1) break;
      for (const link of inside.call(node, "a[href]")) {
        const page = pageOf(attribute.call(link, "href") || "");
        if (page) return page;
      }
    }
    return "";
  };

  /**
   * TikTok's feed has no link to the video playing: its id is in the
   * item's own props (React's, on the elements around the video), and its
   * author in the item's link to «/@user».
   */
  const tiktokPage = (video) => {
    if (!/(^|\.)tiktok\.com$/.test(location.hostname)) return "";
    let id = "";
    let author = "";
    let node = video;
    for (let depth = 0; node && depth < 10 && !(id && author); depth++, node = node.parentElement) {
      if (depth > 0 && inside.call(node, "video").length > 1) break;
      if (!author) {
        for (const link of inside.call(node, 'a[href^="/@"]')) {
          const user = /^\/@([\w.-]{1,64})\/?$/.exec(attribute.call(link, "href") || "");
          if (user) {
            author = user[1];
            break;
          }
        }
      }
      if (id) continue;
      const key = keysOf(node).find((k) => k.startsWith("__reactFiber"));
      for (let fiber = key ? node[key] : null, steps = 0; fiber && steps < 40 && !id; fiber = fiber.return, steps++) {
        const props = fiber.memoizedProps;
        if (props && typeof props.id === "string" && /^\d{15,20}$/.test(props.id)) id = props.id;
      }
    }
    return id && author ? `https://www.tiktok.com/@${author}/video/${id}` : "";
  };

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
        page: linkedPage(video) || tiktokPage(video),
      });
      if (found.length >= 20) break;
    }
    return found;
  };

  let last = "";
  let current = [];
  const report = () => {
    current = videos();
    const message = stringify({
      t: TOKEN,
      title: String(document.title || "").slice(0, 200),
      videos: current.map(({ src, w, h, d, p, page }) => ({ src, w, h, d, p, page })),
    });
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
    addEventListener("resize", soon, { passive: true });
    soon();
  };
  if (document.readyState === "loading") document.addEventListener("DOMContentLoaded", watch, { once: true });
  else watch();
})();
