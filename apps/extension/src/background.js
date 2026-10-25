// @ts-check
// Zajil's browser extension (M5, ADR-0005): downloads of the listed types go
// to Zajil through `zajil-host`, and "Download with Zajil" in a link's menu.
// Every listener is added at once, at the top: a Manifest V3 background
// stops when idle and is woken by these events.

import { api, onNewDownload, pausesSafely } from "./compat.js";
import { menuMessage, needsSignIn, send, shouldCapture, takeOver } from "./capture.js";
import { cookieHeader, cookiesAllowed } from "./cookies.js";
import { load } from "./settings.js";

const MENU = "zajil-download-link";

/** @param {string} key */
const t = (key) => api.i18n.getMessage(key) || key;

/** The notification text for each outcome Zajil did not take: for a
 * captured download (paused first, or not: Firefox), and for a link sent
 * from the menu, where no browser download exists. */
const TEXTS = {
  paused: { refused: "notRefused", unknown: "notAnswered", unreachable: "notReachable" },
  running: { refused: "notRefused", unknown: "notAnsweredRunning", unreachable: "notReachable" },
  menu: { refused: "menuRefused", unknown: "menuNotAnswered", unreachable: "menuNotReachable" },
};

/**
 * Tells the user what happened when Zajil did not take it. A site that
 * wants a sign-in (401 or 403 to Zajil) says so, and offers to allow the
 * cookies (if not allowed) and, for a link from the menu, to download it
 * with the browser.
 *
 * @param {import("./capture.js").Result | null} result
 * @param {keyof typeof TEXTS} where
 * @param {string} [url] the link, for "Download with the browser"
 */
async function report(result, where, url) {
  if (result?.alsoInBrowser) {
    await notify(t("takenAlsoInBrowser"));
    return;
  }
  if (!result || result.outcome === "taken") {
    await api.action.setBadgeText({ text: "" });
    return;
  }
  await api.action.setBadgeText({ text: result.outcome === "unknown" ? "?" : "!" });
  if (needsSignIn(result)) {
    const allowed = await cookiesAllowed(api);
    const menu = where === "menu";
    const text = t(menu ? (allowed ? "menuSignInEvenWithCookies" : "menuSignInNeedsCookies") : allowed ? "signInEvenWithCookies" : "signInNeedsCookies");
    /** @type {{ action: string, title: string }[]} */
    const actions = [];
    if (!allowed) actions.push({ action: "allow", title: t("cookiesAllow") });
    if (menu && url) actions.push({ action: "browser", title: t("downloadWithBrowser") });
    await notify(text, actions, url);
    return;
  }
  await notify(t(TEXTS[where][result.outcome]));
}

/**
 * Shows `message`, with buttons where the browser has them (Chrome and
 * Edge; Firefox has none, so clicking the notification opens the page to
 * allow the cookies). What a button does is kept in `storage.session`, as
 * the background may stop before it is pressed.
 *
 * @param {string} message
 * @param {{ action: string, title: string }[]} [actions]
 * @param {string} [url]
 */
async function notify(message, actions = [], url) {
  const base = { type: "basic", iconUrl: "icons/128.png", title: t("extensionName"), message };
  let id;
  try {
    id = await api.notifications.create(actions.length ? { ...base, buttons: actions.map((a) => ({ title: a.title })) } : base);
  } catch {
    id = await api.notifications.create(base);
  }
  if (actions.length && id) {
    await api.storage.session.set({ [`notice:${id}`]: { actions: actions.map((a) => a.action), url } }).catch(() => {});
  }
}

/** What a notification's button (or, without buttons, the notification)
 * does: open the page that asks for the cookies, or download the link with
 * the browser.
 * @param {string} id @param {number | null} index */
async function act(id, index) {
  const key = `notice:${id}`;
  const notice = (await api.storage.session.get(key).catch(() => ({})))[key];
  if (!notice) return;
  const action = index === null ? notice.actions[0] : notice.actions[index];
  if (action === "allow") await api.tabs.create({ url: api.runtime.getURL("welcome.html") });
  if (action === "browser" && notice.url) await api.downloads.download({ url: notice.url });
  await api.storage.session.remove(key).catch(() => {});
  await api.notifications.clear(id).catch(() => {});
}

api.notifications.onButtonClicked?.addListener((/** @type {string} */ id, /** @type {number} */ index) => void act(id, index));
api.notifications.onClicked.addListener((/** @type {string} */ id) => void act(id, null));

onNewDownload(api, (item) => {
  void (async () => {
    const settings = await load(api);
    if (!shouldCapture(item, settings, api.runtime.id)) return;
    const pause = pausesSafely(api);
    const cookies = await cookieHeader(api, item.finalUrl || item.url);
    await report(await takeOver(api, item, { pause, cookies }), pause ? "paused" : "running");
  })();
});

api.runtime.onInstalled.addListener((/** @type {any} */ details) => {
  api.contextMenus.create({ id: MENU, title: t("menuDownloadLink"), contexts: ["link"] });
  // Once, at install: what the extension does, and the cookie permission
  // behind a button (Chrome wants the user's click to ask for it).
  if (details?.reason === "install") void api.tabs.create({ url: api.runtime.getURL("welcome.html") });
});

api.contextMenus.onClicked.addListener((/** @type {any} */ info) => {
  if (info.menuItemId !== MENU) return;
  void (async () => {
    const cookies = info.linkUrl ? await cookieHeader(api, info.linkUrl) : null;
    const message = menuMessage(info, cookies);
    if (!message) {
      await notify(t("menuNotALink"));
      return;
    }
    await report(await send(api, message), "menu", info.linkUrl);
  })();
});

api.action.onClicked.addListener(() => {
  void api.runtime.openOptionsPage();
});
