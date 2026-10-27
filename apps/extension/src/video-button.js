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
// A video the page plays through EME (DRM: `mediaKeys` set, or an
// `encrypted` event) never gets the button (ADR-0009, section 6). Whether a
// page only asked for EME (`requestMediaKeySystemAccess`) cannot be seen
// from here: that would need a script in the page's own world.
//
// As IDM's (owner's decision, 2026-10-27): at the video's top right, with
// Zajil's mark, in Zajil's own language (the background asks zajil-host,
// which reads Zajil's settings). Pressed, it stays and says it is working
// until Zajil answers, then says if it failed. On YouTube only on a video's
// own page, a Short or an embedded player: not on the home page, search or
// a channel, whose previews play as the pointer passes.

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
  /** How long "sent" and an error stay. */
  const DONE_MS = 1500;
  const ERROR_MS = 6000;
  /** The longest the button waits for Zajil's answer (zajil-host gives up
   * sooner: 20 s to start Zajil, then 10 s on its channel). */
  const ANSWER_MS = 45_000;

  /** Zajil's mark (`assets/icon/zajil-mark.svg`), drawn here: no file of the
   * extension is open to pages (no `web_accessible_resources`). A test checks
   * it is the same as the file. */
  const MARK_VIEWBOX = "44.85 44.40 478.12 478.12";
  const MARK_PATHS = [
    "M485.797 459.805C483.197 461.645 480.467 461.915 477.467 461.465C473.727 460.895 465.257 459.035 463.127 455.775C460.197 451.295 458.157 441.825 459.127 436.635C455.757 432.805 449.617 428.425 445.407 425.195C447.437 425.655 456.347 429.245 457.877 430.535C466.377 437.745 471.317 452.005 480.407 458.085C481.537 458.835 484.857 457.925 486.717 458.355L486.747 458.835ZM490.857 435.475C481.077 417.445 459.907 409.355 441.527 403.035C431.507 399.645 421.417 396.505 411.237 393.615C412.047 390.095 412.657 386.205 413.447 382.575C414.877 376.075 416.547 369.625 418.437 363.245C430.107 322.455 451.617 285.165 481.087 254.655C479.357 252.505 476.647 249.865 474.657 247.855L460.967 234.235L422.987 195.735L349.657 121.425C331.987 103.355 314.187 85.415 296.247 67.615C296.977 74.565 298.177 81.545 299.307 88.455C302.257 106.575 306.017 124.325 311.127 141.975C312.457 146.575 313.627 152.015 315.837 156.235C317.677 159.755 338.217 179.305 342.737 183.795L408.637 249.185L408.307 249.905C401.397 245.975 394.387 241.555 387.527 237.465L347.657 213.695C340.307 209.275 331.977 203.875 324.527 199.805C327.567 211.755 329.917 223.505 333.217 235.635C343.567 274.125 357.477 311.575 374.757 347.505C382.657 363.685 391.297 379.825 400.327 395.395C406.007 405.195 411.967 414.765 417.567 424.625C403.307 410.055 385.307 389.915 373.117 373.685C337.537 326.275 308.007 264.425 294.077 206.645C281.707 198.315 268.267 190.055 255.687 181.765L132.347 100.935L87.237 71.525L75.167 63.595C72.767 62.005 69.227 59.815 67.167 57.935C72.557 78.935 79.347 99.545 87.497 119.635C90.977 128.275 95.857 139.925 100.517 148.035C103.337 149.415 107.767 152.195 110.507 153.855C120.637 160.145 130.517 166.815 140.127 173.835C158.677 187.085 176.997 200.645 195.077 214.495L241.017 249.525C252.097 257.895 265.077 267.325 275.767 275.965C258.787 267.675 241.867 259.235 225.017 250.645L120.917 197.775C123.057 202.975 125.707 207.805 128.217 213.075C137.477 232.475 148.137 251.435 160.657 268.915C161.427 269.985 170.197 274.165 172.017 275.135C175.117 276.795 178.197 278.505 181.237 280.275C202.857 292.785 224.017 308.815 244.097 323.595C263.787 337.915 283.177 352.645 302.257 367.765C292.317 364.215 279.077 357.975 268.927 353.735L189.127 320.265C198.697 333.815 208.547 347.145 218.677 360.275C247.217 397.425 277.767 432.625 312.537 464.175C318.587 469.125 324.377 476.185 331.057 480.145C356.587 495.295 387.667 493.065 412.227 477.165C426.607 468.865 444.367 471.535 459.337 476.745C462.247 477.755 465.767 479.025 468.727 479.615C470.707 475.515 470.907 474.925 474.687 472.285C478.907 469.335 482.467 467.705 487.367 466.195C487.347 465.275 487.297 464.315 487.267 463.385C487.707 463.765 488.167 464.125 488.627 464.485C489.917 465.475 490.837 466.195 492.457 465.995C493.527 465.415 494.027 463.805 494.327 462.615C496.667 453.595 495.227 443.545 490.857 435.475",
    "M408.632 249.187C410.369 250.841 411.014 251.679 408.307 249.904Z",
    "M488.371 474.321C490.952 474.100 492.137 474.299 494.563 474.850C500.662 483.022 499.912 499.344 499.482 508.993C498.340 506.308 497.480 504.240 496.058 501.671C490.422 492.898 485.316 488.999 477.128 482.939C477.404 480.888 477.486 480.254 478.142 478.302C480.993 476.466 485.112 475.165 488.371 474.321",
  ];

  /** YouTube: a video's own page, a Short, or an embedded player only. */
  const YOUTUBE = /(^|\.)(youtube\.com|youtube-nocookie\.com)$/;
  const wantedHere = () => !YOUTUBE.test(location.hostname) || /^\/(watch|shorts\/|embed\/|live\/)/.test(location.pathname);

  const host = document.createElement("div");
  const root = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = `
    :host {
      all: initial !important; display: block !important; opacity: 1 !important;
      visibility: visible !important; filter: none !important;
    }
    button {
      position: fixed; z-index: 2147483647; margin: 0; max-width: 360px;
      display: flex; align-items: center; gap: 6px;
      font: 600 13px/1.3 system-ui, "Segoe UI", "IBM Plex Sans Arabic", sans-serif; color: #fff;
      background: rgba(16, 38, 61, 0.9); border: 1px solid rgba(255, 255, 255, 0.35);
      border-radius: 6px; padding: 5px 10px; cursor: pointer; text-align: start;
    }
    button[hidden] { display: none; }
    button:hover, button:focus-visible { background: rgba(16, 38, 61, 0.98); outline: 2px solid #8adcff; }
    button[data-state="working"] { cursor: progress; }
    button[data-state="error"] { background: rgba(150, 30, 30, 0.95); cursor: default; }
    svg { width: 18px; height: 18px; flex: none; }
    .spin {
      display: none; width: 14px; height: 14px; flex: none; box-sizing: border-box;
      border: 2px solid rgba(255, 255, 255, 0.35); border-top-color: #fff; border-radius: 50%;
      animation: zajil-spin 0.8s linear infinite;
    }
    button[data-state="working"] .spin { display: block; }
    @keyframes zajil-spin { to { transform: rotate(1turn); } }
    @media (prefers-reduced-motion: reduce) { .spin { animation-duration: 2.4s; } }
  `;
  const button = document.createElement("button");
  button.type = "button";
  button.hidden = true;
  button.dataset.state = "idle";
  const SVG = "http://www.w3.org/2000/svg";
  const mark = document.createElementNS(SVG, "svg");
  mark.setAttribute("viewBox", MARK_VIEWBOX);
  mark.setAttribute("fill", "currentColor");
  mark.setAttribute("aria-hidden", "true");
  for (const d of MARK_PATHS) {
    const path = document.createElementNS(SVG, "path");
    path.setAttribute("d", d);
    mark.append(path);
  }
  const spin = document.createElement("span");
  spin.className = "spin";
  const label = document.createElement("span");
  label.setAttribute("aria-live", "polite");
  button.append(mark, spin, label);
  root.append(style, button);

  /** The button's texts: the browser's language until the background gives
   * Zajil's (asked once, at the first video). */
  let texts = {
    dir: api.i18n.getMessage("lang") === "en" ? "ltr" : "rtl",
    button: api.i18n.getMessage("videoButton") || "Zajil",
    working: api.i18n.getMessage("videoButtonWorking") || "…",
    done: api.i18n.getMessage("videoButtonDone") || "✓",
    failed: api.i18n.getMessage("videoButtonFailed") || "✗",
  };
  let asked = false;
  const askTexts = () => {
    if (asked) return;
    asked = true;
    void api.runtime
      .sendMessage({ type: "video-button-texts" })
      .then((/** @type {any} */ answer) => {
        const given = answer?.texts;
        if (!given || typeof given !== "object") return;
        for (const key of ["button", "working", "done", "failed"]) {
          if (typeof given[key] !== "string" || !given[key]) return;
        }
        texts = { dir: answer.lang === "en" ? "ltr" : "rtl", button: given.button, working: given.working, done: given.done, failed: given.failed };
        if (button.dataset.state === "idle") setState("idle");
      })
      .catch(() => {});
  };

  /** @param {"idle" | "working" | "done" | "error"} state @param {string} [text] */
  const setState = (state, text) => {
    button.dataset.state = state;
    button.dir = texts.dir;
    button.setAttribute("aria-busy", String(state === "working"));
    label.textContent = text ?? (state === "working" ? texts.working : state === "done" ? texts.done : texts.button);
  };
  setState("idle");

  /** Videos that fired `encrypted`: played through EME. */
  const encrypted = new WeakSet();
  // Capture, on the document: the event does not bubble.
  document.addEventListener(
    "encrypted",
    (event) => {
      if (event.target instanceof HTMLVideoElement) encrypted.add(event.target);
    },
    { capture: true },
  );
  /** @param {HTMLVideoElement} found */
  const isProtected = (found) => found.mediaKeys != null || encrypted.has(found);

  /** @type {HTMLVideoElement | null} */
  let video = null;
  /** @type {ReturnType<typeof setTimeout> | undefined} */
  let hiding;
  /** Pressed, and not yet back to rest: it stays on its video. */
  const busy = () => button.dataset.state !== "idle";

  const hide = () => {
    if (busy()) return;
    button.hidden = true;
    video = null;
  };

  /** At the video's top right, inside the window. @param {HTMLVideoElement} found */
  const place = (found) => {
    const box = found.getBoundingClientRect();
    // The viewport's width without its scrollbar, also in quirks mode.
    const width = (document.scrollingElement ?? document.documentElement).clientWidth;
    button.style.top = `${Math.max(box.top, 0) + 8}px`;
    button.style.right = `${Math.max(width - Math.min(box.right, width), 0) + 8}px`;
    button.style.left = "auto";
  };

  /** @param {HTMLVideoElement} found */
  const show = (found) => {
    if (busy()) return;
    const box = found.getBoundingClientRect();
    if (box.width < MIN_WIDTH || box.height < MIN_HEIGHT || isProtected(found) || !wantedHere()) return;
    askTexts();
    // Last in the page each time, above what the page added since.
    if (host !== document.documentElement.lastElementChild) document.documentElement.append(host);
    video = found;
    place(found);
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
      // A move the page makes shows nothing, and so asks nothing (review of
      // #261: it made the extension start zajil-host).
      if (!event.isTrusted) return;
      if (event.composedPath().includes(host)) {
        clearTimeout(hiding);
        return;
      }
      if (!pending) requestAnimationFrame(look);
      pending = { x: event.clientX, y: event.clientY };
    },
    { passive: true, capture: true },
  );
  // Pressed, it follows its video while the video is there (else it stays
  // where it was); at rest, it goes.
  addEventListener("scroll", () => (busy() && video?.isConnected ? place(video) : hide()), { passive: true, capture: true });

  /** Back to rest after `ms`, then gone after `LINGER_MS` unless the
   * pointer moves on it meanwhile. @param {number} ms */
  const rest = (ms) => {
    setTimeout(() => {
      setState("idle");
      clearTimeout(hiding);
      hiding = setTimeout(hide, LINGER_MS);
    }, ms);
  };

  button.addEventListener("click", async (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (busy() || !event.isTrusted || !video || isProtected(video)) return;
    // Not pressed for real if the page made it unseen.
    // Both spellings: `opacityProperty` and `visibilityProperty` came in
    // Chrome 121, the older names work from Chrome 105 (review).
    const seen = host.checkVisibility(
      /** @type {any} */ ({ opacityProperty: true, visibilityProperty: true, checkOpacity: true, checkVisibilityCSS: true }),
    );
    if (!seen) return;
    const src = video.currentSrc || video.src || "";
    setState("working");
    /** @type {any} */
    let answer = null;
    try {
      // A longer address is left out, never cut into another one (review).
      answer = await Promise.race([
        api.runtime.sendMessage({ type: "video-button", src: src.length <= 8192 ? src : "" }),
        new Promise((done) => setTimeout(() => done(null), ANSWER_MS)),
      ]);
    } catch {
      // The extension was updated or stopped under this page.
    }
    if (answer?.taken === true) {
      setState("done");
      rest(DONE_MS);
    } else {
      const why = typeof answer?.error === "string" && answer.error ? answer.error : texts.failed;
      setState("error", why);
      rest(ERROR_MS);
    }
  });
})();
