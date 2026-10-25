// @ts-check
// Zajil's browser extension (M5, ADR-0005): downloads of the listed types go
// to Zajil through `zajil-host`, and "Download with Zajil" in a link's menu.
// Every listener is added at once, at the top: a Manifest V3 background
// stops when idle and is woken by these events.

import { api, onNewDownload, pausesSafely } from "./compat.js";
import { PROTOCOL_VERSION, menuMessage, needsSignIn, send, shouldCapture, takeOver, videoMessage } from "./capture.js";
import { cookieHeader, cookiesAllowed } from "./cookies.js";
import { load } from "./settings.js";
import { instanceId, newTicket, plainStore, rememberTicket, startLink } from "./link.js";

const MENU = "zajil-download-link";
const MENU_VIDEO = "zajil-download-video";

/** @param {string} key */
const t = (key) => api.i18n.getMessage(key) || key;

/** The notification text for each outcome Zajil did not take: for a
 * captured download (paused first, or not: Firefox), and for a link sent
 * from the menu, where no browser download exists. */
const TEXTS = {
  paused: { refused: "notRefused", unknown: "notAnswered", unreachable: "notReachable" },
  running: { refused: "notRefused", unknown: "notAnsweredRunning", unreachable: "notReachable" },
  menu: { refused: "menuRefused", unknown: "menuNotAnswered", unreachable: "menuNotReachable" },
  video: { refused: "videoRefused", unknown: "videoNotAnswered", unreachable: "menuNotReachable" },
};

/**
 * Tells the user what happened when Zajil did not take it. A site that
 * wants a sign-in (401 or 403 to Zajil) says so, and offers to allow the
 * cookies (if not allowed) and, for a link from the menu, to download it
 * with the browser.
 *
 * @param {import("./capture.js").Result | null} result
 * @param {keyof typeof TEXTS} where
 * @param {{ url?: string, sentCookies?: boolean, inPrivate?: boolean, store?: string }} [about]
 *   the link, for "Download with the browser"; whether cookies went with
 *   it; whether it came from a private window; its tab's cookie store (a
 *   Firefox container)
 */
async function report(result, where, about = {}) {
  const { url, sentCookies = false, inPrivate = false, store } = about;
  if (result?.alsoInBrowser) {
    await notify(t("takenAlsoInBrowser"));
    return;
  }
  if (!result || result.outcome === "taken") {
    await api.action.setBadgeText({ text: "" });
    return;
  }
  await api.action.setBadgeText({ text: result.outcome === "unknown" ? "?" : "!" });
  // A video goes to yt-dlp, which says no 401 or 403 of its own; and the
  // browser cannot download a page's video in its place.
  if (where !== "video" && needsSignIn(result)) {
    // "Even with the cookies" only when cookies went (review of #76: with
    // the permission, a store that gave none, or a header left out, sent
    // nothing).
    const allowed = await cookiesAllowed(api);
    const menu = where === "menu";
    const keys = sentCookies
      ? ["menuSignInEvenWithCookies", "signInEvenWithCookies"]
      : allowed
        ? ["menuSignInNoCookies", "signInNoCookies"]
        : ["menuSignInNeedsCookies", "signInNeedsCookies"];
    const text = t(menu ? keys[0] : keys[1]);
    /** @type {{ action: string, title: string }[]} */
    const actions = [];
    // From a private window, no button (security review #84): "Download
    // with the browser" would download it in the normal profile, with its
    // cookies, into its history; and nothing is kept for a button.
    if (!inPrivate && !allowed) actions.push({ action: "allow", title: t("cookiesAllow") });
    if (!inPrivate && menu && url) actions.push({ action: "browser", title: t("downloadWithBrowser") });
    await notify(text, actions, url, store);
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
 * @param {string} [store] the tab's cookie store, so the browser downloads
 *   it in the same container
 */
async function notify(message, actions = [], url, store) {
  const base = { type: "basic", iconUrl: "icons/128.png", title: t("extensionName"), message };
  let id;
  try {
    id = await api.notifications.create(actions.length ? { ...base, buttons: actions.map((a) => ({ title: a.title })) } : base);
  } catch {
    id = await api.notifications.create(base);
  }
  if (actions.length && id) {
    await api.storage.session.set({ [`notice:${id}`]: { actions: actions.map((a) => a.action), url, store } }).catch(() => {});
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
  // In the tab's own container (Firefox), not the default one (review #84).
  if (action === "browser" && notice.url) {
    await api.downloads.download(notice.store ? { url: notice.url, cookieStoreId: notice.store } : { url: notice.url });
  }
  await api.storage.session.remove(key).catch(() => {});
  await api.notifications.clear(id).catch(() => {});
}

api.notifications.onButtonClicked?.addListener((/** @type {string} */ id, /** @type {number} */ index) => void act(id, index));
api.notifications.onClicked.addListener((/** @type {string} */ id) => void act(id, null));
// A notice closed without a choice: what it would have done is forgotten.
api.notifications.onClosed?.addListener((/** @type {string} */ id) => {
  void api.storage.session.remove(`notice:${id}`).catch(() => {});
});

onNewDownload(api, (item) => {
  void (async () => {
    const settings = await load(api);
    if (!shouldCapture(item, settings, api.runtime.id)) return;
    const pause = pausesSafely(api);
    const url = item.finalUrl || item.url;
    const cookies = await cookieHeader(api, url, item.cookieStoreId);
    const ticket = newTicket();
    const result = await takeOver(api, item, { pause, cookies, instance: await instanceId(api), ticket });
    // While it is open, Zajil may ask for its cookies again after a restart.
    if (cookies && result?.outcome === "taken") await rememberTicket(api, ticket, url, plainStore(item.cookieStoreId));
    await report(result, pause ? "paused" : "running", { sentCookies: Boolean(cookies) });
  })();
});

api.runtime.onInstalled.addListener((/** @type {any} */ details) => {
  api.contextMenus.create({ id: MENU, title: t("menuDownloadLink"), contexts: ["link"] });
  api.contextMenus.create({ id: MENU_VIDEO, title: t("menuDownloadVideo"), contexts: ["video", "audio"] });
  // Once, at install: what the extension does, and the cookie permission
  // behind a button (Chrome wants the user's click to ask for it). A flag,
  // since an extension loaded unpacked is "installed" at every start.
  if (details?.reason === "install") {
    void (async () => {
      const { welcomed } = await api.storage.local.get("welcomed").catch(() => ({}));
      if (welcomed) return;
      await api.storage.local.set({ welcomed: true }).catch(() => {});
      await api.tabs.create({ url: api.runtime.getURL("welcome.html") });
    })();
  }
});

api.contextMenus.onClicked.addListener((/** @type {any} */ info, /** @type {any} */ tab) => {
  if (info.menuItemId === MENU_VIDEO) {
    void sendVideo(info, tab);
    return;
  }
  if (info.menuItemId !== MENU) return;
  void (async () => {
    // The link is checked before any cookie is read (review of #76).
    if (!menuMessage(info)) {
      await notify(t("menuNotALink"));
      return;
    }
    // A private window's cookies are never sent: the link goes without.
    const cookies = tab?.incognito ? null : await cookieHeader(api, info.linkUrl, tab?.cookieStoreId);
    const from = {
      // Nothing that ties a private window's download to this browser.
      instance: tab?.incognito ? undefined : await instanceId(api),
      store: tab?.incognito ? undefined : plainStore(tab?.cookieStoreId),
      ticket: newTicket(),
    };
    const message = /** @type {Record<string, unknown>} */ (menuMessage(info, cookies, from));
    // Zajil keeps nothing of a private window's download once it is done.
    if (tab?.incognito) message.private = true;
    const result = await send(api, message);
    if (cookies && result.outcome === "taken") await rememberTicket(api, from.ticket, info.linkUrl, from.store);
    // Zajil is told when the last private window closes (only a flag).
    if (tab?.incognito && result.outcome === "taken") await api.storage.session.set({ privateSent: true }).catch(() => {});
    await report(result, "menu", {
      url: info.linkUrl,
      sentCookies: Boolean(cookies),
      inPrivate: Boolean(tab?.incognito),
      store: tab?.incognito ? undefined : plainStore(tab?.cookieStoreId),
    });
  })();
});

/**
 * "Download video with Zajil" (M6): the video's page goes to Zajil, which
 * shows its formats. The page's cookies go with it when the user allowed
 * them, never from a private window; it is checked before any is read.
 * @param {any} info @param {any} tab
 */
async function sendVideo(info, tab) {
  const isPrivate = Boolean(tab?.incognito);
  const page = videoMessage(info);
  if (!page) {
    await notify(t("videoNotWeb"));
    return;
  }
  const cookies = isPrivate ? null : await cookieHeader(api, /** @type {string} */ (page.page), tab?.cookieStoreId);
  const message = /** @type {Record<string, unknown>} */ (videoMessage(info, { cookies, private: isPrivate }));
  const result = await send(api, message);
  if (isPrivate && result.outcome === "taken") await api.storage.session.set({ privateSent: true }).catch(() => {});
  await report(result, "video");
}

// The last private window closed: Zajil takes its downloads out of its
// queues and deletes what they left (review of #91). Only if one was sent
// to it, so Zajil is not started for nothing.
api.windows?.onRemoved.addListener(() => {
  void (async () => {
    const { privateSent } = await api.storage.session.get("privateSent").catch(() => ({}));
    if (!privateSent) return;
    const open = await api.windows.getAll().catch(() => []);
    if (open.some((/** @type {any} */ w) => w.incognito)) return;
    await api.storage.session.remove("privateSent").catch(() => {});
    await send(api, { type: "private_closed", protocol_version: PROTOCOL_VERSION });
  })();
});

api.action.onClicked.addListener(() => {
  void api.runtime.openOptionsPage();
});

// The link to Zajil, open while the browser is (M5): each time the
// background starts, it opens it again. A background starts lazily, only
// for an event it listens to: this one starts it with the browser (review
// of #80: the link never opened at startup).
api.runtime.onStartup.addListener(() => {});
startLink(api);
