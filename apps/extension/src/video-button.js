// @ts-check
// The "Download video" button over a page's videos (M6, owner's decision).
// Only with "Allow Zajil on sites": the background registers this script on
// every site then, and removes it when the permission goes.
//
// It reads nothing of the page but where its videos are and their address.
// A press sends the background only that address: the page, its frame and
// its tab come from the browser (`runtime.MessageSender`), not from here.
// The button lives in a closed shadow root, so the page can neither see it
// nor restyle it, and only a real click (`isTrusted`) counts.
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
    :host { all: initial; }
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
    if (!host.isConnected) document.documentElement.append(host);
    video = found;
    button.style.top = `${Math.max(box.top, 0) + 8}px`;
    button.style.left = `${Math.max(box.left, 0) + 8}px`;
    button.hidden = false;
    clearTimeout(hiding);
  };

  // The pointer's video, even under the player's own controls: the
  // elements under the pointer, not only the top one.
  document.addEventListener(
    "pointermove",
    (event) => {
      if (event.composedPath().includes(host)) {
        clearTimeout(hiding);
        return;
      }
      const found = document.elementsFromPoint(event.clientX, event.clientY).find((e) => e instanceof HTMLVideoElement);
      if (found) {
        show(/** @type {HTMLVideoElement} */ (found));
      } else if (!button.hidden) {
        clearTimeout(hiding);
        hiding = setTimeout(hide, LINGER_MS);
      }
    },
    { passive: true, capture: true },
  );
  addEventListener("scroll", hide, { passive: true, capture: true });

  button.addEventListener("click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted || !video) return;
    const src = video.currentSrc || video.src || "";
    hide();
    void api.runtime.sendMessage({ type: "video-button", src: src.slice(0, 8192) }).catch(() => {});
  });
})();
