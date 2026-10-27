// @ts-check
// Zajil's browser extension (M5, ADR-0005): downloads of the listed types go
// to Zajil through `zajil-host`, and "Download with Zajil" in a link's menu.
// Every listener is added at once, at the top: a Manifest V3 background
// stops when idle and is woken by these events.

import { api, onNewDownload, pausesSafely, withDownloadUiHidden } from "./compat.js";
import { HOST, HOST_PROBLEMS, PROTOCOL_VERSION, isPageRefusal, isWeb, menuMessage, needsSignIn, send, shouldCapture, takeOver, videoMessage } from "./capture.js";
import { cookieHeader, cookiesAllowed, otherSite, videoAllowed } from "./cookies.js";
import { load } from "./settings.js";
import { forgetTicket, instanceId, newTicket, plainStore, rememberTicket, startLink } from "./link.js";
import { forget, forgetAll, note, originOf, streamFor } from "./streams.js";

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
  // zajil-host's own failure: in Zajil's language, with no detail of the
  // system (#264).
  if (where === "video" && result.code) {
    await notify(await hostText(result.code));
    return;
  }
  // A video Zajil refuses: its own answer, in the user's language («Video
  // downloads are coming soon», until M6; review of #99: the user saw only
  // "did not take this video").
  if (where === "video" && result.outcome === "refused" && result.error) {
    await notify(result.error.slice(0, 300));
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

/** After Zajil answered a download sent with cookies: taken, its ticket
 * starts its life now; refused or unreachable, it is forgotten. An answer
 * that never came («unknown») keeps it: Zajil may have taken it, and its
 * `keep` forgets it otherwise (review of the Drive links).
 * @param {{ outcome: string } | null | undefined} result @param {string} ticket
 * @param {string} url @param {string | undefined} store
 * @param {{ noStrict?: boolean }} [how] the ticket's (`rememberTicket`) */
async function settleTicket(result, ticket, url, store, how = {}) {
  if (result?.outcome === "taken") await rememberTicket(api, ticket, url, store, undefined, how);
  else if (result?.outcome === "refused" || result?.outcome === "unreachable") await forgetTicket(api, ticket);
}

/** How long a link clicked with the browser's keys stays the browser's. */
const LEAVE_MS = 30_000;

/** The links left to the browser (`leave` in `storage.session`) are read
 * and written one change at a time, in the order the events came (review
 * of #169: a click and its download arriving together, or two clicks,
 * lost one another). */
let leaveQueue = Promise.resolve();

/** Changes the list of links left to the browser, after every change
 * asked before; expired ones are dropped.
 * @template T
 * @param {(live: { url: string, until: number }[]) => T} change
 * @returns {Promise<T>} */
function changeLeave(change) {
  const done = leaveQueue.then(async () => {
    const { leave = [] } = await api.storage.session.get("leave").catch(() => ({}));
    const now = Date.now();
    const live = leave.filter((/** @type {any} */ l) => l.until > now);
    const result = change(live);
    await api.storage.session.set({ leave: live.slice(-20) }).catch(() => {});
    return result;
  });
  leaveQueue = done.then(
    () => {},
    () => {},
  );
  return done;
}

/** Whether `item` is a link the user clicked with the browser's keys
 * (`click-keys.js`) a moment ago: then it is the browser's, once.
 * @param {any} item */
function leftToBrowser(item) {
  return changeLeave((live) => {
    const at = live.findIndex((l) => l.url === item.url || l.url === item.finalUrl);
    if (at >= 0) live.splice(at, 1);
    return at >= 0;
  });
}

// The browser's download UI back at every start of the background: one
// stopped in the middle of a hand-over left it hidden (review of #253;
// Chrome forgets it only when the extension is unloaded). Before any
// listener can hide it again.
void api.downloads.setUiOptions?.({ enabled: true })?.catch(() => {});

onNewDownload(api, async (item, ready) => {
  const settings = await load(api);
  if (!shouldCapture(item, settings, api.runtime.id)) return;
  if (await leftToBrowser(item)) return;
  // Hidden before Chrome settles the download (`ready`), so the browser
  // does not show a download Zajil takes; back once it is handed over.
  await withDownloadUiHidden(api, () => {
    ready();
    return handOver(item);
  });
});

/**
 * Asks Zajil to take a captured browser download, with its cookies when
 * allowed, and says what came of it.
 *
 * @param {any} item a `downloads.DownloadItem`
 */
async function handOver(item) {
  const pause = pausesSafely(api);
  const url = item.finalUrl || item.url;
  // Strict cookies (`SameSite=Strict`) go only as the browser sends them:
  // from a page of the same site (#92: a link from another site, which
  // the browser fetches without them, got them in Zajil). With no page
  // to tell by (no referrer), none.
  const page = typeof item.referrer === "string" && item.referrer ? item.referrer : undefined;
  const strict = { from: page, noStrict: page === undefined };
  const cookies = await cookieHeader(api, url, item.cookieStoreId, strict);
  const ticket = newTicket();
  // While it is open, Zajil may ask for its cookies again after a
  // restart, and for its file's download address while taking it (a
  // Google Drive link): kept before Zajil is asked, without the strict
  // ones when they did not go now.
  const how = { noStrict: page === undefined || otherSite(page, url) };
  if (cookies) await rememberTicket(api, ticket, url, plainStore(item.cookieStoreId), undefined, how);
  const result = await takeOver(api, item, { pause, cookies, instance: await instanceId(api), ticket });
  if (cookies) await settleTicket(result, ticket, url, plainStore(item.cookieStoreId), how);
  await report(result, pause ? "paused" : "running", { sentCookies: Boolean(cookies) });
}

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
  void sendLink(info, tab);
});

/**
 * A link to Zajil: from its menu, or clicked with Zajil's keys
 * (`click-keys.js`). Its cookies go with it when the user allowed them,
 * never from a private window. From the keys (owner's decision): its
 * `SameSite=Strict` cookies only from a page of the same site, as the
 * browser; and a link Zajil finds to be a web page opens in a tab behind
 * this one, as Ctrl+click would have.
 * @param {any} info the link (`linkUrl`) and its page (`pageUrl`) @param {any} tab
 * @param {{ keys?: boolean }} [how]
 */
async function sendLink(info, tab, how = {}) {
  // The link is checked before any cookie is read (review of #76).
  if (!menuMessage(info)) {
    await notify(t("menuNotALink"));
    return;
  }
  // A private window's cookies are never sent: the link goes without.
  const cookies = tab?.incognito ? null : await cookieHeader(api, info.linkUrl, tab?.cookieStoreId, how.keys ? { from: info.pageUrl } : {});
  const from = {
    // Nothing that ties a private window's download to this browser.
    instance: tab?.incognito ? undefined : await instanceId(api),
    store: tab?.incognito ? undefined : plainStore(tab?.cookieStoreId),
    ticket: newTicket(),
  };
  const message = /** @type {Record<string, unknown>} */ (menuMessage(info, cookies, from));
  // Zajil keeps nothing of a private window's download once it is done.
  if (tab?.incognito) message.private = true;
  // Kept before Zajil is asked (it may ask for a Drive file's download
  // address meanwhile).
  const ticket = { noStrict: Boolean(how.keys) && otherSite(info.pageUrl, info.linkUrl) };
  if (cookies) await rememberTicket(api, from.ticket, info.linkUrl, from.store, undefined, ticket);
  const result = await send(api, message);
  if (cookies) await settleTicket(result, from.ticket, info.linkUrl, from.store, ticket);
  // Zajil is told when the last private window closes (only a flag).
  if (tab?.incognito && result.outcome === "taken") await api.storage.session.set({ privateSent: true }).catch(() => {});
  if (how.keys && isPageRefusal(result)) {
    await openBehind(info.linkUrl, tab);
    return;
  }
  await report(result, "menu", {
    url: info.linkUrl,
    sentCookies: Boolean(cookies),
    inPrivate: Boolean(tab?.incognito),
    store: tab?.incognito ? undefined : plainStore(tab?.cookieStoreId),
  });
}

/** Opens `url` in a new tab behind `tab`, next to it, in its window and
 * its container (Firefox): where Ctrl+click would have opened it.
 * @param {string} url @param {any} tab */
async function openBehind(url, tab) {
  /** @type {Record<string, unknown>} */
  const where = { url, active: false };
  if (Number.isSafeInteger(tab?.windowId)) where.windowId = tab.windowId;
  if (Number.isSafeInteger(tab?.index)) where.index = tab.index + 1;
  if (Number.isSafeInteger(tab?.id)) where.openerTabId = tab.id;
  if (typeof tab?.cookieStoreId === "string") where.cookieStoreId = tab.cookieStoreId;
  await api.tabs.create(where).catch(() => api.tabs.create({ url, active: false }).catch(() => {}));
  await api.action.setBadgeText({ text: "" }).catch(() => {});
}

/**
 * "Download video with Zajil" (M6): the video's page goes to Zajil, which
 * shows its formats. The page's cookies go with it when the user allowed
 * them, never from a private window; it is checked before any is read.
 * From the menu, what came of it is a notification; from the button
 * (`quiet`), the button says it, so nothing more is shown.
 * @param {any} info @param {any} tab
 * @param {string} [doc] the document that asked (the button's sender), so
 *   only its own playlist goes
 * @param {boolean} [quiet]
 * @returns {Promise<import("./capture.js").Result | "not-web">}
 */
async function sendVideo(info, tab, doc, quiet = false) {
  const isPrivate = Boolean(tab?.incognito);
  const page = videoMessage(info);
  if (!page) {
    if (!quiet) await notify(t("videoNotWeb"));
    return "not-web";
  }
  const cookies = isPrivate ? null : await cookieHeader(api, /** @type {string} */ (page.page), tab?.cookieStoreId);
  // The playlist the tab loaded for it (nothing is noted in a private one).
  // Only while the permission lasts (review: none noted before it went).
  const noted = !isPrivate && Number.isSafeInteger(tab?.id) && (await videoAllowed(api));
  const want = { frame: Number.isSafeInteger(info.frameId) ? info.frameId : 0, doc, origin: originOf(info.frameUrl ?? info.pageUrl) };
  const stream = noted ? await streamFor(api, tab.id, want) : undefined;
  const message = /** @type {Record<string, unknown>} */ (videoMessage(info, { cookies, private: isPrivate, stream }));
  const result = await send(api, message);
  if (isPrivate && result.outcome === "taken") await api.storage.session.set({ privateSent: true }).catch(() => {});
  if (!quiet) await report(result, "video");
  // From the button: a "!" left by an earlier failure goes once it worked.
  else if (result.outcome === "taken") await api.action.setBadgeText({ text: "" }).catch(() => {});
  return result;
}

/** How long Zajil's language is kept before zajil-host is asked again. */
const LANGUAGE_MS = 60_000;

/** The ask for Zajil's language under way, so asks at once share it. */
/** @type {Promise<"ar" | "en"> | null} */
let askingLanguage = null;

/**
 * Zajil's own language, "ar" or "en", for the video button (owner's
 * decision, 2026-10-27): zajil-host says it in its answer to `ping`, from
 * Zajil's settings. Asked at most once a minute, and once for asks that
 * come together (each ask starts zajil-host; review of #261: a page of many
 * frames asked many at once); the browser's language when Zajil does not
 * say.
 * @returns {Promise<"ar" | "en">}
 */
async function zajilLanguage() {
  const { zajilLanguage: kept } = await api.storage.session.get("zajilLanguage").catch(() => ({}));
  if ((kept?.lang === "ar" || kept?.lang === "en") && Date.now() - kept.at < LANGUAGE_MS) return kept.lang;
  askingLanguage ??= (async () => {
    /** @type {"ar" | "en" | undefined} */
    let lang;
    try {
      const answer = await api.runtime.sendNativeMessage(HOST, { type: "ping", protocol_version: PROTOCOL_VERSION });
      if (answer?.language === "ar" || answer?.language === "en") lang = answer.language;
    } catch {
      // Zajil is not there: the browser's language.
    }
    lang ??= t("lang") === "en" ? "en" : "ar";
    await api.storage.session.set({ zajilLanguage: { lang, at: Date.now() } }).catch(() => {});
    return lang;
  })().finally(() => {
    askingLanguage = null;
  });
  return askingLanguage;
}

/** Each language's texts, read once from the extension's own files. */
const localeTexts = new Map();

/**
 * The text of `key` in `lang`, from `_locales/<lang>/messages.json`, not
 * the browser's language (`i18n.getMessage` knows only that one).
 * @param {"ar" | "en"} lang @param {string} key
 */
async function textIn(lang, key) {
  if (!localeTexts.has(lang)) {
    // A failed read is not kept: the next ask reads again (review of #261).
    const read = fetch(api.runtime.getURL(`_locales/${lang}/messages.json`))
      .then((r) => (r.ok ? r.json() : Promise.reject(new Error(String(r.status)))))
      .catch(() => {
        localeTexts.delete(lang);
        return {};
      });
    localeTexts.set(lang, read);
  }
  const all = await localeTexts.get(lang);
  return typeof all?.[key]?.message === "string" ? all[key].message : t(key);
}

/** What zajil-host's own failure `code` says, in Zajil's language.
 * @param {string} code */
async function hostText(code) {
  const key = HOST_PROBLEMS[/** @type {keyof typeof HOST_PROBLEMS} */ (code)] ?? "hostUnreachable";
  return textIn(await zajilLanguage(), key);
}

/** The button's texts, in Zajil's language. */
async function buttonTexts() {
  const lang = await zajilLanguage();
  const [button, working, done, failed] = await Promise.all(
    ["videoButton", "videoButtonWorking", "videoButtonDone", "videoButtonFailed"].map((key) => textIn(lang, key)),
  );
  return { lang, texts: { button, working, done, failed } };
}

/**
 * What the button says when Zajil did not take the video: Zajil's own
 * answer when it refused (already in its language), else the extension's
 * text for it, in Zajil's language.
 * @param {import("./capture.js").Result | "not-web"} result
 */
async function buttonAnswer(result) {
  if (result !== "not-web" && result.outcome === "taken") return { taken: true };
  if (result !== "not-web" && result.code) return { taken: false, error: await hostText(result.code) };
  if (result !== "not-web" && result.outcome === "refused" && result.error) return { taken: false, error: result.error.slice(0, 300) };
  const lang = await zajilLanguage();
  const key = result === "not-web" ? "videoNotWeb" : TEXTS.video[/** @type {"refused" | "unknown" | "unreachable"} */ (result.outcome)];
  return { taken: false, error: await textIn(lang, key) };
}

// The "Download video" button (`video-button.js`) was pressed: the page,
// its frame and its tab are the browser's word (the sender), never the
// message's; only the video's address comes from the page. The button
// waits for the answer (`sendResponse`, kept open by returning true).
api.runtime.onMessage.addListener((/** @type {any} */ message, /** @type {any} */ sender, /** @type {(answer: unknown) => void} */ sendResponse) => {
  if (sender?.id !== api.runtime.id || !sender.tab) return;
  if (message?.type === "click-browser") {
    // Noted at once, before any wait, so its download finds it (review of
    // #169). Leaving a link to the browser needs no permission; and nothing
    // of a private window is kept (its downloads are never taken anyway).
    if (!sender.tab.incognito && isWeb(message.href)) {
      const url = /** @type {string} */ (message.href);
      void changeLeave((live) => void live.push({ url, until: Date.now() + LEAVE_MS }));
    }
    return;
  }
  if (message?.type === "click-link") {
    void clicked(message.href, sender);
    return;
  }
  if (message?.type === "video-button-texts") {
    void buttonTexts().then(sendResponse, () => sendResponse(null));
    return true;
  }
  if (message?.type !== "video-button") return;
  const frameId = Number.isSafeInteger(sender.frameId) ? sender.frameId : 0;
  const info = {
    pageUrl: sender.tab.url,
    frameUrl: frameId === 0 ? undefined : sender.url,
    srcUrl: typeof message.src === "string" ? message.src : undefined,
    frameId,
  };
  void sendVideo(info, sender.tab, typeof sender.documentId === "string" ? sender.documentId : undefined, true)
    .then(buttonAnswer)
    .then(sendResponse, () => sendResponse(null));
  return true;
});

/**
 * A link clicked with Zajil's keys goes to Zajil as from its menu, its page
 * the browser's word (the frame's own address). Only while "Allow Zajil on
 * sites" lasts and the keys are on (review: a script left from before does
 * nothing).
 * @param {unknown} href @param {any} sender
 */
async function clicked(href, sender) {
  if (typeof href !== "string" || !(await videoAllowed(api)) || !(await load(api)).useKeys) return;
  await sendLink({ linkUrl: href, pageUrl: sender.url ?? sender.tab.url }, sender.tab, { keys: true });
}

/** The button's script, on every site while the video permissions last. */
const BUTTON = {
  id: "zajil-video-button",
  js: ["video-button.js"],
  matches: ["<all_urls>"],
  allFrames: true,
  runAt: "document_idle",
  persistAcrossSessions: true,
};

/** The keys of a click (`click-keys.js`), on every site and from the
 * start of a page, under the same permission. */
const CLICKS = {
  id: "zajil-click-keys",
  js: ["click-keys.js"],
  matches: ["<all_urls>"],
  allFrames: true,
  runAt: "document_start",
  persistAcrossSessions: true,
};

/** @param {any} details */
const onHeaders = (details) => void note(api, details);
const WATCHED = { urls: ["<all_urls>"], types: ["main_frame", "xmlhttprequest", "media", "other"] };

/** Starts or stops the video features as the permissions now are: the
 * button's script registered or not, the playlists watched or not. */
async function syncVideo() {
  const allowed = await videoAllowed(api);
  // The keys' scripts already in open tabs stay there when the permission
  // goes (unregistering is for pages loaded later): they read this, and
  // take no click while it is not true (review of #169).
  await api.storage.local.set({ clickKeysOn: allowed }).catch(() => {});
  const requests = api.webRequest?.onHeadersReceived;
  if (allowed && requests && !requests.hasListener(onHeaders)) {
    try {
      // Watching only: no "blocking".
      requests.addListener(onHeaders, WATCHED, ["responseHeaders"]);
    } catch {
      // the permission went meanwhile
    }
  }
  if (!allowed && requests?.hasListener(onHeaders)) requests.removeListener(onHeaders);
  if (!allowed) await forgetAll(api);
  if (!api.scripting) return;
  for (const script of [BUTTON, CLICKS]) {
    const registered = await api.scripting.getRegisteredContentScripts({ ids: [script.id] }).catch(() => []);
    if (allowed && !registered.length) await api.scripting.registerContentScripts([script]).catch(() => {});
    if (!allowed && registered.length) await api.scripting.unregisterContentScripts({ ids: [script.id] }).catch(() => {});
  }
}

// Added at once, at the top, when the permission is there: a background
// stopped meanwhile is woken by the next playlist.
if (api.webRequest?.onHeadersReceived) {
  try {
    api.webRequest.onHeadersReceived.addListener(onHeaders, WATCHED, ["responseHeaders"]);
  } catch {
    // not allowed
  }
}
api.permissions?.onAdded?.addListener(() => void syncVideo());
api.permissions?.onRemoved?.addListener(() => void syncVideo());
api.tabs.onRemoved?.addListener((/** @type {number} */ tab) => void forget(api, tab));
void syncVideo();

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
