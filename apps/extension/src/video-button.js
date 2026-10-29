// @ts-check
// The "Download video" button over a page's videos (M6, owner's decision).
// Only with "Allow Zajil on sites": the background registers this script on
// every site then, and removes it when the permission goes.
//
// It reads nothing of the page but where its videos are and their address.
// A press sends the background only that address: the page, its frame and
// its tab come from the browser (`runtime.MessageSender`), not from here.
// So does the button's first showing on a page (`video-peek`), for Zajil
// to read the video ahead.
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
// As in other download managers (owner's decision, 2026-10-27): at the video's top right, with
// Zajil's mark, in Zajil's own language (the background asks zajil-host,
// which reads Zajil's settings). Pressed, it stays and says it is working
// until Zajil answers, then says if it failed. On YouTube only on a video's
// own page, a Short or an embedded player: not on the home page, search or
// a channel, whose previews play as the pointer passes.
//
// YouTube's playlists (owner's decision, 2026-10-28): a playlist's page has
// no video, so it gets a button of its own, always there, beside the
// playlist's title: "Download the playlist with Zajil". And on a video
// played from a playlist (`/watch?v=…&list=…`), the button offers two
// choices: "This video" and "The whole playlist". Both send the background
// only what was chosen: the playlist's page comes from the tab's own
// address (`playlistPage` in capture.js), not from here. Same language,
// same "Sending…", same protections (a closed shadow root, `isTrusted`). A
// Mix (`list=RD…`) is no playlist: YouTube makes it as you watch.
//
// The pill and its list (owner's decision, 2026-10-29): on YouTube the
// button says how many formats the video has, «Download video (N) ▾», and
// a press opens their list at once, each with its size and rate: read from
// the player's own answer in the page (`yt-page.js`, in the page's world,
// asked by an event on the document), never waiting for Zajil. A choice
// sends the background only that format's itag (and its sound's); Zajil
// finds it in what it read itself. «?» opens Zajil's page in the browser,
// «✕» hides the button for this video. Elsewhere, or before the page says
// anything, a press sends the video as before.
//
// Updated, reloaded or turned off, the extension leaves this script in the
// open tabs, cut off from it: `runtime.id` is gone, and `sendMessage`
// throws at once ("Extension context invalidated"), not a refused promise.
// So every ask goes through `ask`, and every listener first checks the
// extension is there; once it is not, the button and all the listeners go,
// for good (`stop`).

(() => {
  const api = /** @type {any} */ (globalThis).browser ?? /** @type {any} */ (globalThis).chrome;
  const flag = "__zajilVideoButton";
  if (!api?.runtime?.id || /** @type {any} */ (globalThis)[flag]) return;
  /** @type {any} */ (globalThis)[flag] = true;

  /** Whether the extension is still there. */
  const alive = () => {
    try {
      return Boolean(api.runtime?.id);
    } catch {
      return false;
    }
  };
  /** Each listener's removal, for `stop`. @type {(() => void)[]} */
  const added = [];
  /** Listens while the extension is there: the first event after it went
   * stops the script instead.
   * @param {EventTarget} target @param {string} type @param {(event: any) => void} listener
   * @param {boolean | AddEventListenerOptions} [options] */
  const listen = (target, type, listener, options) => {
    const checked = (/** @type {Event} */ event) => (alive() ? listener(event) : stop());
    target.addEventListener(type, checked, options);
    added.push(() => target.removeEventListener(type, checked, options));
  };
  let stopped = false;
  /** The extension went: the button and every listener go. */
  const stop = () => {
    if (stopped) return;
    stopped = true;
    for (const remove of added.splice(0)) remove();
    clearTimeout(hiding);
    host.remove();
    listHost.remove();
  };
  /** Asks the background; null when it gives no answer. Once the extension
   * went, null at once, and the script stops.
   * @param {object} message @returns {Promise<any>} */
  const ask = (message) => {
    const gone = () => {
      if (!alive()) stop();
      return null;
    };
    if (!alive()) return Promise.resolve(gone());
    try {
      return Promise.resolve(api.runtime.sendMessage(message)).catch(gone);
    } catch {
      return Promise.resolve(gone());
    }
  };

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
  /** A YouTube playlist here, at `path` (`/playlist`, or `/watch` for a
   * video played from it), in the top frame: as `playlistPage` in
   * capture.js, which a content script cannot import.
   * @param {string} path */
  const LIST_SITE = /^(www\.|m\.)?youtube\.com$/;
  const LIST_ID = /^(?!RD(?!CLAK5uy_)|TL)[\w-]{2,200}$/;
  const listHere = (/** @type {string} */ path) =>
    window === window.top &&
    location.protocol === "https:" &&
    LIST_SITE.test(location.hostname) &&
    location.pathname === path &&
    LIST_ID.test(new URLSearchParams(location.search).get("list") ?? "");

  const host = document.createElement("div");
  const root = host.attachShadow({ mode: "closed" });
  const style = document.createElement("style");
  style.textContent = `
    :host {
      all: initial !important; display: block !important; opacity: 1 !important;
      visibility: visible !important; filter: none !important;
    }
    .bar, .menu { position: fixed; z-index: 2147483647; }
    .bar { display: flex; flex-direction: row-reverse; align-items: center; gap: 4px; direction: ltr; }
    .bar:has(.main[hidden]) { display: none; }
    button {
      margin: 0; max-width: 360px;
      display: flex; align-items: center; gap: 6px;
      font: 600 13px/1.3 system-ui, "Segoe UI", "IBM Plex Sans Arabic", "Noto Sans Arabic", sans-serif; color: #fff;
      background: rgba(15, 15, 18, 0.82); border: 1px solid rgba(255, 255, 255, 0.22);
      border-radius: 999px; padding: 6px 12px; cursor: pointer; text-align: start;
      box-shadow: 0 2px 10px rgba(0, 0, 0, 0.35); backdrop-filter: blur(6px);
      transition: background 0.12s;
    }
    button[hidden] { display: none; }
    button:hover, button:focus-visible { background: rgba(10, 10, 12, 0.95); outline: none; border-color: rgba(255, 255, 255, 0.55); }
    button:focus-visible { outline: 2px solid #8adcff; }
    button[data-state="working"] { cursor: progress; }
    button[data-state="done"] { background: rgba(20, 110, 60, 0.92); }
    button[data-state="error"] { background: rgba(150, 30, 30, 0.95); cursor: default; }
    .main svg { width: 16px; height: 16px; flex: none; }
    .main .arrow { width: 10px; height: 10px; opacity: 0.85; }
    .main .arrow[hidden] { display: none; }
    .small { width: 26px; height: 26px; padding: 0; justify-content: center; font-size: 13px; }
    .small[hidden] { display: none; }
    .spin {
      display: none; width: 14px; height: 14px; flex: none; box-sizing: border-box;
      border: 2px solid rgba(255, 255, 255, 0.35); border-top-color: #fff; border-radius: 50%;
      animation: zajil-spin 0.8s linear infinite;
    }
    button[data-state="working"] .spin { display: block; }
    button[data-state="working"] .mark { display: none; }
    @keyframes zajil-spin { to { transform: rotate(1turn); } }
    @media (prefers-reduced-motion: reduce) { .spin { animation-duration: 2.4s; } }
    .menu {
      display: flex; flex-direction: column; gap: 1px; padding: 4px; min-width: 230px; max-width: 380px;
      max-height: min(70vh, 460px); overflow-y: auto; box-sizing: border-box;
      background: rgba(22, 22, 26, 0.97); border: 1px solid rgba(255, 255, 255, 0.18); border-radius: 10px;
      box-shadow: 0 8px 28px rgba(0, 0, 0, 0.5); color: #fff;
      font: 500 12.5px/1.35 system-ui, "Segoe UI", "IBM Plex Sans Arabic", "Noto Sans Arabic", sans-serif;
      scrollbar-width: thin; scrollbar-color: rgba(255,255,255,0.3) transparent;
    }
    .menu[hidden] { display: none; }
    .menu button {
      width: 100%; max-width: none; background: transparent; border-color: transparent; border-radius: 6px;
      box-shadow: none; backdrop-filter: none; padding: 6px 8px; justify-content: space-between; gap: 12px;
      font-weight: 500;
    }
    .menu button:hover, .menu button:focus-visible { background: rgba(255, 255, 255, 0.1); border-color: transparent; }
    .menu button.item { padding: 4px 8px; }
    .menu .what { display: flex; align-items: baseline; gap: 8px; min-width: 0; white-space: nowrap; }
    .menu .name { font-weight: 650; }
    .menu .detail { opacity: 0.6; font-size: 11.5px; }
    .menu .size { opacity: 0.9; font-variant-numeric: tabular-nums; white-space: nowrap; font-weight: 600; }
    .menu .name, .menu .detail, .menu .size { direction: ltr; unicode-bidi: isolate; }
    .menu .head { padding: 6px 8px 4px; opacity: 0.6; font-size: 11px; font-weight: 600; letter-spacing: 0.02em; }
    .menu .title { display: flex; gap: 8px; padding: 6px 8px; font-weight: 650; border-bottom: 1px solid rgba(255,255,255,0.1); margin-bottom: 2px; }
    .menu .title .words { flex: 1; min-width: 0; white-space: nowrap; overflow: hidden; text-overflow: ellipsis; unicode-bidi: plaintext; }
    .menu .title .length { flex: none; opacity: 0.6; font-weight: 500; direction: ltr; unicode-bidi: isolate; }
    .menu hr { border: 0; border-top: 1px solid rgba(255, 255, 255, 0.1); margin: 3px 4px; width: auto; }
  `;
  const SVG = "http://www.w3.org/2000/svg";
  /** An icon. @param {string} viewBox @param {string[]} paths @param {string} name its class
   * @param {boolean} [stroke] drawn by lines, not filled */
  const icon = (viewBox, paths, name, stroke = false) => {
    const made = document.createElementNS(SVG, "svg");
    made.setAttribute("viewBox", viewBox);
    made.setAttribute("class", name);
    made.setAttribute("aria-hidden", "true");
    if (stroke) {
      for (const [key, value] of [["fill", "none"], ["stroke", "currentColor"], ["stroke-width", "2.2"], ["stroke-linecap", "round"], ["stroke-linejoin", "round"]]) made.setAttribute(key, value);
    } else made.setAttribute("fill", "currentColor");
    for (const d of paths) {
      const path = document.createElementNS(SVG, "path");
      path.setAttribute("d", d);
      made.append(path);
    }
    return made;
  };
  /** A download arrow over a tray. */
  const DOWNLOAD = ["M12 4v11", "M7 10.5l5 5 5-5", "M5 19.5h14"];
  /** A button with its icon (Zajil's mark, or a download arrow), its
   * spinner and its text; the video's has an arrow for its list.
   * @param {string} name its class @param {"mark" | "download"} [kind] */
  const makeButton = (name, kind = "mark") => {
    const made = document.createElement("button");
    made.type = "button";
    made.className = name;
    made.dataset.state = "idle";
    const mark = kind === "mark" ? icon(MARK_VIEWBOX, MARK_PATHS, "mark") : icon("0 0 24 24", DOWNLOAD, "mark", true);
    const spin = document.createElement("span");
    spin.className = "spin";
    const text = document.createElement("span");
    text.setAttribute("aria-live", "polite");
    made.append(mark, spin, text);
    return { button: made, label: text };
  };
  const main = { ...makeButton("main", "download"), rest: (/** @type {Texts} */ t) => (choices.length ? `${t.button} (${choices.length})` : t.button) };
  const button = main.button;
  button.hidden = true;
  const arrow = icon("0 0 24 24", ["M6 9l6 6 6-6"], "arrow", true);
  arrow.setAttribute("hidden", "");
  button.append(arrow);
  /** «?» and «✕», beside the button. @param {string} text */
  const small = (text) => {
    const made = document.createElement("button");
    made.type = "button";
    made.className = "small";
    made.textContent = text;
    return made;
  };
  const help = small("?");
  const close = small("✕");
  const bar = document.createElement("div");
  bar.className = "bar";
  bar.append(button, help, close);
  // On a video played from a playlist: its two choices, under the button.
  const menu = document.createElement("div");
  menu.className = "menu";
  menu.hidden = true;
  menu.setAttribute("role", "menu");
  const choice = () => {
    const made = document.createElement("button");
    made.type = "button";
    made.setAttribute("role", "menuitem");
    return made;
  };
  const thisVideo = choice();
  thisVideo.dataset.act = "this";
  const wholeList = choice();
  wholeList.dataset.act = "whole";
  menu.append(thisVideo, wholeList);
  root.append(style, bar, menu);

  /** The video's formats, as the page's player says them (`yt-page.js`,
   * made into choices by `yt-formats.js`); none elsewhere.
   * @type {{ title: string, duration: string, video: any[], audio: any[] } | null} */
  let listing = null;
  /** Every choice of `listing`, in the list's order. @type {any[]} */
  let choices = [];
  /** What the page's world answered last (a text, checked when read). */
  let pageSaid = "";
  listen(document, "zajil-yt-formats", (event) => {
    if (typeof event.detail === "string" && event.detail.length <= 512 * 1024) pageSaid = event.detail;
  });
  /** Asks the page's world now (it answers at once, during the ask). */
  const readPage = () => {
    const make = /** @type {any} */ (globalThis).__zajilFormats?.choicesOf;
    if (typeof make !== "function" || !YOUTUBE.test(location.hostname)) return;
    pageSaid = "";
    document.dispatchEvent(new CustomEvent("zajil-yt-ask"));
    let data = null;
    try {
      data = JSON.parse(pageSaid || "null");
    } catch {
      data = null;
    }
    listing = make(data);
    choices = listing ? [...listing.video, ...listing.audio] : [];
    arrow.toggleAttribute("hidden", !choices.length && !listHere("/watch"));
    if (!busy()) setState("idle");
  };

  /** One line of the list. @param {string} cls @param {string} words */
  const line = (cls, words) => {
    const made = document.createElement("div");
    made.className = cls;
    made.textContent = words;
    return made;
  };
  /** The list, made again for what is known now. */
  const fillMenu = () => {
    menu.replaceChildren();
    if (texts) menu.dir = texts.dir;
    if (!listing || !texts) {
      menu.append(thisVideo, wholeList);
      return;
    }
    const title = line("title", "");
    title.append(line("words", listing.title));
    if (listing.duration) title.append(line("length", listing.duration));
    menu.append(title);
    const group = (/** @type {string} */ head, /** @type {any[]} */ items, /** @type {number} */ from) => {
      if (!items.length) return;
      menu.append(line("head", head));
      items.forEach((item, i) => {
        const made = choice();
        made.dataset.act = "format";
        made.className = "item";
        made.dataset.index = String(from + i);
        const what = document.createElement("span");
        what.className = "what";
        what.append(line("name", item.name), line("detail", item.detail));
        const size = document.createElement("span");
        size.className = "size";
        size.textContent = item.size;
        made.append(what, size);
        menu.append(made);
      });
    };
    group(texts.listVideo ?? "Video", listing.video, 0);
    group(texts.listAudio ?? "Audio", listing.audio, listing.video.length);
    menu.append(document.createElement("hr"));
    if (listHere("/watch")) menu.append(wholeList);
    thisVideo.textContent = texts.more ?? texts.thisVideo;
    menu.append(thisVideo);
  };

  // A playlist's page: its own button, in a host of its own beside the
  // title, in the page's flow (not over it).
  const listHost = document.createElement("div");
  const listRoot = listHost.attachShadow({ mode: "closed" });
  const listStyle = document.createElement("style");
  listStyle.textContent = `${style.textContent}
    :host { margin: 8px 0 !important; }
    .page { width: fit-content; }
  `;
  const page = { ...makeButton("page"), rest: (/** @type {Texts} */ t) => t.playlist };
  listRoot.append(listStyle, page.button);

  /** The buttons' texts, in Zajil's language, from the background (asked
   * once: at the first video, or on a playlist's page at the first real
   * move of the pointer or key press, never by the page alone). Until they come the button is not shown at
   * all, so it never shows in another language first (owner's decision on
   * #264); if they never come, it never shows.
   * @typedef {{ dir: string, button: string, working: string, done: string, failed: string, playlist: string, thisVideo: string, wholeList: string, listVideo?: string, listAudio?: string, more?: string, help?: string, hide?: string }} Texts
   * @type {Texts | null} */
  let texts = null;
  /** The video the pointer was on while the texts were on their way. */
  /** @type {HTMLVideoElement | null} */
  let waiting = null;
  let asked = false;
  const askTexts = () => {
    if (asked) return;
    asked = true;
    void ask({ type: "video-button-texts" }).then((/** @type {any} */ answer) => {
      const given = answer?.texts;
      if (stopped || !given || typeof given !== "object") return;
      const keys = /** @type {const} */ (["button", "working", "done", "failed", "playlist", "thisVideo", "wholeList"]);
      for (const key of keys) {
        if (typeof given[key] !== "string" || !given[key]) return;
      }
      const dir = answer.lang === "en" ? "ltr" : "rtl";
      texts = /** @type {Texts} */ ({ dir, ...Object.fromEntries(keys.map((key) => [key, given[key]])) });
      // The list's words, when this Zajil has them.
      for (const key of /** @type {const} */ (["listVideo", "listAudio", "more", "help", "hide"])) {
        if (typeof given[key] === "string" && given[key]) texts[key] = given[key];
      }
      help.title = texts.help ?? "Zajil";
      close.title = texts.hide ?? "✕";
      close.setAttribute("aria-label", close.title);
      setState("idle");
      setState("idle", undefined, page);
      menu.dir = dir;
      thisVideo.textContent = texts.more ?? texts.thisVideo;
      wholeList.textContent = texts.wholeList;
      const found = waiting;
      waiting = null;
      if (found?.isConnected) show(found);
      syncList();
    });
  };

  /** Only once the texts came (the buttons show only then).
   * @param {"idle" | "working" | "done" | "error"} state @param {string} [text]
   * @param {typeof main} [which] the video's button, or the playlist page's */
  const setState = (state, text, which = main) => {
    if (!texts) return;
    which.button.dataset.state = state;
    which.button.dir = texts.dir;
    which.button.setAttribute("aria-busy", String(state === "working"));
    which.label.textContent = text ?? (state === "working" ? texts.working : state === "done" ? texts.done : which.rest(texts));
  };

  /** Videos that fired `encrypted`: played through EME. */
  const encrypted = new WeakSet();
  // Capture, on the document: the event does not bubble.
  listen(
    document,
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

  const closeMenu = () => {
    menu.hidden = true;
    button.removeAttribute("aria-expanded");
  };
  /** Under the button, at the same right edge, inside the window. */
  const placeMenu = () => {
    const below = button.getBoundingClientRect().bottom + 4;
    menu.style.top = `${below}px`;
    menu.style.maxHeight = `${Math.max(window.innerHeight - below - 12, 120)}px`;
    menu.style.right = bar.style.right;
    menu.style.left = "auto";
  };

  const hide = () => {
    if (busy()) return;
    closeMenu();
    button.hidden = true;
    video = null;
  };

  /** At the video's top right, inside the window. @param {HTMLVideoElement} found */
  const place = (found) => {
    const box = found.getBoundingClientRect();
    // The viewport's width without its scrollbar, also in quirks mode.
    const width = (document.scrollingElement ?? document.documentElement).clientWidth;
    bar.style.top = `${Math.max(box.top, 0) + 8}px`;
    bar.style.right = `${Math.max(width - Math.min(box.right, width), 0) + 8}px`;
    bar.style.left = "auto";
    if (!menu.hidden) placeMenu();
  };

  /** @param {HTMLVideoElement} found */
  const show = (found) => {
    if (stopped || busy()) return;
    const box = found.getBoundingClientRect();
    if (box.width < MIN_WIDTH || box.height < MIN_HEIGHT || isProtected(found) || !wantedHere() || location.href === dismissed) return;
    if (!texts) {
      waiting = found;
      askTexts();
      return;
    }
    // Last in the page each time, above what the page added since.
    if (host !== document.documentElement.lastElementChild) document.documentElement.append(host);
    if (found !== video) closeMenu();
    video = found;
    place(found);
    button.hidden = false;
    clearTimeout(hiding);
    peek(found);
  };

  /** The page whose button «✕» hid. */
  let dismissed = "";
  /** The page whose video was last sent to be read ahead. */
  let peeked = "";
  /** Zajil reads the video ahead as soon as its button shows, once a page
   * (owner's decision, 2026-10-28, as in other download managers): its qualities are there when
   * the button is pressed. Only the video's address goes, as for a press.
   * @param {HTMLVideoElement} found */
  const peek = (found) => {
    if (location.href === peeked) return;
    peeked = location.href;
    readPage();
    const src = found.currentSrc || found.src || "";
    void ask({ type: "video-peek", src: src.length <= 8192 ? src : "" });
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
    if (!at || stopped) return;
    const found = document.elementsFromPoint(at.x, at.y).find((e) => e instanceof HTMLVideoElement);
    if (found) {
      show(/** @type {HTMLVideoElement} */ (found));
    } else if (!texts) {
      // Left before the texts came: nothing to show then.
      waiting = null;
    } else if (!button.hidden) {
      clearTimeout(hiding);
      hiding = setTimeout(hide, LINGER_MS);
    }
  };

  listen(
    document,
    "pointermove",
    (event) => {
      // A move the page makes shows nothing, and so asks nothing (review of
      // #261: it made the extension start zajil-host).
      if (!event.isTrusted) return;
      if (!asked && listHere("/playlist")) askTexts();
      if (event.composedPath().includes(host)) {
        clearTimeout(hiding);
        return;
      }
      if (!pending) requestAnimationFrame(look);
      pending = { x: event.clientX, y: event.clientY };
    },
    { passive: true, capture: true },
  );
  // The pointer left the page while the texts were on their way: nothing
  // to show when they come.
  listen(document, "pointerout", (event) => {
    if (!event.relatedTarget) waiting = null;
  });
  // Pressed, it follows its video while the video is there (else it stays
  // where it was); at rest, it goes.
  listen(window, "scroll", () => (busy() && video?.isConnected ? place(video) : hide()), { passive: true, capture: true });

  /** Back to rest after `ms`, then gone after `LINGER_MS` unless the
   * pointer moves on it meanwhile. @param {number} ms */
  const rest = (ms) => {
    setTimeout(() => {
      setState("idle");
      clearTimeout(hiding);
      hiding = setTimeout(hide, LINGER_MS);
    }, ms);
  };

  /** Whether the page left the button seen (else no press counts).
   * Both spellings: `opacityProperty` and `visibilityProperty` came in
   * Chrome 121, the older names work from Chrome 105 (review).
   * @param {HTMLElement} which its host */
  const seen = (which) =>
    which.checkVisibility(/** @type {any} */ ({ opacityProperty: true, visibilityProperty: true, checkOpacity: true, checkVisibilityCSS: true }));

  /** The background's answer to a press, or null after `ANSWER_MS`. Gone
   * while it waited (its answer never came): the script stops.
   * @param {object} message */
  const answerTo = async (message) => {
    /** @type {any} */
    const answer = await Promise.race([ask(message), new Promise((done) => setTimeout(() => done(null), ANSWER_MS))]);
    if (!stopped && !alive()) stop();
    return answer;
  };
  /** What a press that did not work says. @param {any} answer */
  const why = (answer) => (typeof answer?.error === "string" && answer.error ? answer.error : (texts?.failed ?? "✗"));

  /** A real press, on a button the page left seen, over its video. @param {Event} event */
  const pressable = (event) => {
    event.preventDefault();
    event.stopPropagation();
    return !busy() && event.isTrusted && video != null && !isProtected(video) && seen(host);
  };

  /** Sends the video, or the whole playlist it is played from, or one of
   * its formats (its itag, and its sound's).
   * @param {boolean} whole @param {{ id?: string, video?: string, audio?: string, label?: string, bytes?: number }} [format] */
  const press = async (whole, format) => {
    if (!video) return;
    const src = video.currentSrc || video.src || "";
    setState("working");
    // A longer address is left out, never cut into another one (review).
    /** @type {Record<string, unknown>} */
    const message = whole ? { type: "video-button", whole: true } : { type: "video-button", src: src.length <= 8192 ? src : "" };
    if (format) message.format = format;
    const answer = await answerTo(message);
    if (stopped) return;
    if (answer?.taken === true) {
      setState("done");
      rest(DONE_MS);
    } else {
      setState("error", why(answer));
      rest(ERROR_MS);
    }
  };

  listen(button, "click", (event) => {
    if (!pressable(event)) return;
    // The page may have said more since.
    if (menu.hidden) readPage();
    // Its formats, or (played from a playlist) the two choices, first.
    if (!choices.length && !listHere("/watch")) void press(false);
    else if (!menu.hidden) closeMenu();
    else {
      fillMenu();
      menu.hidden = false;
      button.setAttribute("aria-expanded", "true");
      placeMenu();
    }
  });
  listen(menu, "click", (event) => {
    const item = /** @type {HTMLElement | null} */ (event.target instanceof Element ? event.target.closest("button") : null);
    if (!item || menu.hidden || !pressable(event)) return;
    closeMenu();
    const act = item.dataset.act;
    if (act === "whole") void press(true);
    else if (act === "this") void press(false);
    else if (act === "format") {
      const picked = choices[Number(item.dataset.index)];
      if (picked) void press(false, { ...picked.send, label: `${picked.name} · ${picked.detail}`, bytes: picked.bytes });
    }
  });
  listen(help, "click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (event.isTrusted) void ask({ type: "video-help" });
  });
  listen(close, "click", (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted || busy()) return;
    dismissed = location.href;
    hide();
  });
  listen(document, "keydown", (event) => {
    if (event.key === "Escape") closeMenu();
    if (event.isTrusted && !asked && listHere("/playlist")) askTexts();
  });

  /** Where a playlist's title is, by YouTube's pages of today first, then
   * older ones, then any heading. */
  const TITLES = [
    "yt-page-header-view-model h1",
    "ytd-playlist-header-renderer yt-dynamic-sizing-formatted-string",
    "ytd-playlist-header-renderer h1",
    "ytd-playlist-sidebar-primary-info-renderer #title",
    "h1",
  ];
  /** @param {Element} found */
  const visible = (found) => found.checkVisibility() && found.getBoundingClientRect().width > 0;
  /** The playlist page the page's button is for, and its press under way: a
   * press for another page says nothing on this one. */
  let listFor = "";
  let listPress = 0;

  /** The playlist page's button, after the title's box, while the tab is on
   * a playlist's page (YouTube changes pages without loading them). */
  const syncList = () => {
    if (stopped) return;
    if (!listHere("/playlist")) {
      listHost.remove();
      return;
    }
    // Shown once the texts came: asked at the user's first move here.
    if (!texts) return;
    if (location.href !== listFor) {
      listFor = location.href;
      listPress += 1;
      setState("idle", undefined, page);
    }
    /** @type {Element | undefined} */
    let title;
    for (const selector of TITLES) {
      title = [...document.querySelectorAll(selector)].find(visible);
      if (title) break;
    }
    if (!title) {
      listHost.remove();
      return;
    }
    const box = title.closest("yt-dynamic-text-view-model, yt-dynamic-sizing-formatted-string") ?? title;
    if (box.nextElementSibling !== listHost) box.after(listHost);
  };

  listen(page.button, "click", async (event) => {
    event.preventDefault();
    event.stopPropagation();
    if (!event.isTrusted || !texts || page.button.dataset.state !== "idle" || !listHere("/playlist") || !seen(listHost)) return;
    listPress += 1;
    const mine = listPress;
    setState("working", undefined, page);
    const answer = await answerTo({ type: "playlist-button" });
    if (stopped || mine !== listPress) return;
    const taken = answer?.taken === true;
    setState(taken ? "done" : "error", taken ? undefined : why(answer), page);
    setTimeout(() => mine === listPress && setState("idle", undefined, page), taken ? DONE_MS : ERROR_MS);
  });

  // On YouTube's own site, in the top frame: each change of the page, once
  // a frame, looks for a playlist's page.
  if (window === window.top && LIST_SITE.test(location.hostname)) {
    let queued = false;
    const later = () => {
      if (queued) return;
      queued = true;
      requestAnimationFrame(() => {
        queued = false;
        syncList();
      });
    };
    const watcher = new MutationObserver(() => (alive() ? later() : stop()));
    watcher.observe(document.documentElement, { childList: true, subtree: true });
    added.push(() => watcher.disconnect());
    listen(window, "popstate", later);
    later();
  }
})();
