// @ts-check
// The "Download video" button over a page's videos (M6, owner's decision).
// Only with "Allow Zajil on sites": the background registers this script on
// every site then, and removes it when the permission goes.
//
// It reads nothing of the page but where its videos are and their address.
// A press sends the background only that address: the page, its frame and
// its tab come from the browser (`runtime.MessageSender`), not from here.
//
// What the page can and cannot do (review): it cannot reach the button (a
// closed shadow root) nor forge a press on it (only a real click,
// `isTrusted`, counts). Its own CSS cannot hide or restyle the button (the
// root's rules are `!important`, and those of a shadow tree win); a page
// that hides everything (`opacity: 0` on its root) gets no press at all
// (`checkVisibility` at the click). But a page can tell the button's host
// element is there, so that the extension is installed with site access;
// and like any page element, it can be covered.
//
// Its look is a first version: the final one comes with the video window's
// design (M5.5).

(() => {
  const api = /** @type {any} */ (globalThis).browser ?? /** @type {any} */ (globalThis).chrome;
  const flag = "__zajilVideoButton";
  if (!api?.runtime?.id || /** @type {any} */ (globalThis)[flag]) return;
  /** @type {any} */ (globalThis)[flag] = true;

  /** The smallest video that gets the button (not an icon or a preview). */
  const MIN_WIDTH = 200;
  const MIN_HEIGHT = 120;
  /** How long the button stays once the pointer has left the video. */
  const LINGER_MS = 1500;

  const host = document.createElement("div");
  const root = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = `
    :host {
      all: initial !important; display: block !important; opacity: 1 !important;
      visibility: visible !important; filter: none !important;
    }
    button {
      position: fixed; z-index: 2147483647; margin: 0;
      font: 600 13px/1.2 system-ui, sans-serif; color: #fff;
      background: rgba(20, 20, 20, 0.85); border: 1px solid rgba(255, 255, 255, 0.35);
      border-radius: 6px; padding: 6px 10px; cursor: pointer;
    }
    button:hover, button:focus-visible { background: rgba(20, 20, 20, 0.95); outline: 2px solid #4da3ff; }
  `;
  const button = document.createElement("button");
  button.type = "button";
  button.hidden = true;
  button.textContent = api.i18n.getMessage("videoButton") || "Zajil";
  button.dir = api.i18n.getMessage("lang") === "en" ? "ltr" : "rtl";
  root.append(style, button);

  /** @type {HTMLVideoElement | null} */
  let video = null;
  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let hiding;

  const hide = () => {
    button.hidden = true;
    video = null;
  };

  /** @param {HTMLVideoElement} found */
  const show = (found) => {
    const box = found.getBoundingClientRect();
    if (box.width < MIN_WIDTH || box.height < MIN_HEIGHT) return;
    // Last in the page each time, above what the page added since.
    if (host !== document.documentElement.lastElementChild) document.documentElement.append(host);
    video = found;
    button.style.top = `${Math.max(box.top, 0) + 8}px`;
    button.style.left = `${Math.max(box.left, 0) + 8}px`;
    button.hidden = false;
    clearTimeout(hiding);
  };

  /** The last pointer position not yet looked at, once per frame (review:
   * looking at every move forces a layout on every page). */
  /** @type {{ x: number, y: number } | null} */
  let pending = null;

  // The pointer's video, even under the player's own controls: the
  // elements under the pointer, not only the top one.
  const look = () => {
    const at = pending;
    pending = null;
    if (!at) return;
    const found = document.elementsFromPoint(at.x, at.y).find((e) => e instanceof HTMLVideoElement);
    if (found) {
      show(/** @type {HTMLVideoElement} */ (found));
    } else if (!button.hidden) {
      clearTimeout(hiding);
      hiding = setTimeout(hide, LINGER_MS);
    }
  };

  document.addEventListener(
    "pointermove",
    (event) => {
      if (event.composedPath().includes(host)) {
        clearTimeout(hiding);
        return;
      }
      if (!pending) requestAnimationFrame(look);
      pending = { x: event.clientX, y: event.clientY };
    },
    { passive: true, capture: true },
  );
  addEventListener("scroll", hide, { passive: true, capture: true });

  button.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted || !video) return;
    // Not pressed for real if the page made it unseen.
    if (!host.checkVisibility({ opacityProperty: true, visibilityProperty: true })) return;
    const src = video.currentSrc || video.src || "";
    hide();
    // A longer address is left out, never cut into another one (review).
    void api.runtime.sendMessage({ type: "video-button", src: src.length <= 8192 ? src : "" }).catch(() => {});
  });
})();
