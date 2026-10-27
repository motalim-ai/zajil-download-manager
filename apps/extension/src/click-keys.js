// @ts-check
// The keys of a click on a link (owner's decision, 2026-10-26): held while
// clicking, one set sends the link to Zajil, the other leaves it to the
// browser, which then downloads it without Zajil taking it. Off until the
// user turns them on in the extension's settings; then by default Ctrl and
// Alt on Windows and Linux, Option and Option+Shift on a Mac, which the
// settings change. Only with "Allow Zajil on sites": the
// background registers this script on every site then, and removes it when
// the permission goes.
//
// It reads nothing of the page but the clicked link's address. Only a real
// click counts (`isTrusted`): a page cannot send a link by itself. The
// page, its frame and its tab go with the link from the browser
// (`runtime.MessageSender`), not from here.
//
// Updated, reloaded or turned off, the extension leaves this script in the
// open tabs, cut off from it: `runtime.id` is gone, and its APIs throw at
// once, not as a refused promise. So each call to them is in a `try`, and
// once the extension is gone the script removes its listeners and stops.

(() => {
  const api = /** @type {any} */ (globalThis).browser ?? /** @type {any} */ (globalThis).chrome;
  const flag = "__zajilClickKeys";
  if (!api?.runtime?.id || /** @type {any} */ (globalThis)[flag]) return;
  /** @type {any} */ (globalThis)[flag] = true;

  // As in settings.js (`KEY_CHOICES`, `isMac`, `defaultKeys`,
  // `clickKeys`), which a content script cannot import.
  const CHOICES = ["off", "ctrl", "alt", "ctrl+shift", "alt+shift", "ctrl+alt"];
  const nav = /** @type {any} */ (navigator);
  const mac = /^mac/i.test(nav.userAgentData?.platform || nav.platform || "");
  const base = mac ? { send: "alt", browser: "alt+shift" } : { send: "ctrl", browser: "alt" };
  let keys = { ...base };
  // Off until the background says the permission is there (`clickKeysOn`):
  // a script left in an open tab after the permission went, or before the
  // settings are read, takes no click (review of #169).
  let on = false;
  // And only once the user turned the keys on in the settings: off by
  // default, as in IDM (owner's decision).
  let used = false;

  /** @param {any} stored the `settings` in `storage.local` */
  const use = (stored) => {
    const pick = (/** @type {unknown} */ v, /** @type {string} */ fallback) =>
      typeof v === "string" && CHOICES.includes(v) ? v : fallback;
    const send = pick(stored?.sendKeys, base.send);
    const browser = pick(stored?.browserKeys, base.browser);
    keys = { send, browser: browser === send ? "off" : browser };
    used = stored?.useKeys === true;
  };
  try {
    api.storage?.local
      ?.get(["settings", "clickKeysOn"])
      .then((/** @type {any} */ got) => {
        use(got?.settings);
        on = got?.clickKeysOn === true;
      })
      .catch(() => {});
  } catch {
    // cut off already: the first click stops the script
  }
  /** @param {any} changes @param {string} area */
  const changed = (changes, area) => {
    if (area !== "local") return;
    if (changes.settings) use(changes.settings.newValue);
    if (changes.clickKeysOn) on = changes.clickKeysOn.newValue === true;
  };
  try {
    api.storage?.onChanged?.addListener(changed);
  } catch {
    // as above
  }

  /** Whether the extension is still there. */
  const alive = () => {
    try {
      return Boolean(api.runtime?.id);
    } catch {
      return false;
    }
  };

  /** The keys held, as in `CHOICES`; null with Cmd or the Windows key.
   * @param {MouseEvent} event */
  const held = (event) => {
    if (event.metaKey) return null;
    const names = [];
    if (event.ctrlKey) names.push("ctrl");
    if (event.altKey) names.push("alt");
    if (event.shiftKey) names.push("shift");
    return names.join("+");
  };

  /** The http(s) link the click is on, if any: HTML's, or an SVG one.
   * @param {MouseEvent} event */
  const linkOf = (event) => {
    for (const node of event.composedPath()) {
      let href = null;
      if (node instanceof HTMLAnchorElement || node instanceof HTMLAreaElement) href = node.href;
      else if (node instanceof SVGAElement) href = node.href.baseVal;
      else continue;
      if (!href) return null;
      try {
        const url = new URL(href, /** @type {Node} */ (node).baseURI);
        return url.protocol === "http:" || url.protocol === "https:" ? url.href : null;
      } catch {
        return null;
      }
    }
    return null;
  };

  /** The extension went: its listeners go, and the script does nothing
   * more. */
  const stop = () => {
    window.removeEventListener("click", click, true);
    try {
      api.storage?.onChanged?.removeListener(changed);
    } catch {
      // cut off: the browser drops it anyway
    }
  };

  /** @param {MouseEvent} event */
  const click = (event) => {
    if (!alive()) {
      stop();
      return;
    }
    if (!on || !used || !event.isTrusted || event.button !== 0) return;
    const combo = held(event);
    if (!combo) return;
    const which = combo === keys.send ? "click-link" : combo === keys.browser ? "click-browser" : null;
    if (!which) return;
    const href = linkOf(event);
    if (!href) return;
    if (which === "click-link") {
      event.preventDefault();
      event.stopImmediatePropagation();
    }
    try {
      void api.runtime.sendMessage({ type: which, href }).catch(() => {});
    } catch {
      // cut off between the check and here
      if (!alive()) stop();
    }
  };
  // Before the page's own listeners (capture, on the window), so a page
  // that opens its links itself does not open this one too.
  window.addEventListener("click", click, true);
})();
