// @ts-check
// Zajil's browser extension (M5, ADR-0005): downloads of the listed types go
// to Zajil through `zajil-host`, and "Download with Zajil" in a link's menu.
// Every listener is added at once, at the top: a Manifest V3 background
// stops when idle and is woken by these events.

import { api, onNewDownload, pausesSafely } from "./compat.js";
import { menuMessage, send, shouldCapture, takeOver } from "./capture.js";
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
 * Tells the user what happened when Zajil did not take it.
 * @param {import("./capture.js").Result | null} result
 * @param {keyof typeof TEXTS} where
 */
async function report(result, where) {
  if (result?.alsoInBrowser) {
    await notify(t("takenAlsoInBrowser"));
    return;
  }
  if (!result || result.outcome === "taken") {
    await api.action.setBadgeText({ text: "" });
    return;
  }
  await api.action.setBadgeText({ text: result.outcome === "unknown" ? "?" : "!" });
  await notify(t(TEXTS[where][result.outcome]));
}

/** @param {string} message */
async function notify(message) {
  await api.notifications.create({ type: "basic", iconUrl: "icons/128.png", title: t("extensionName"), message });
}

onNewDownload(api, (item) => {
  void (async () => {
    const settings = await load(api);
    if (!shouldCapture(item, settings, api.runtime.id)) return;
    const pause = pausesSafely(api);
    await report(await takeOver(api, item, { pause }), pause ? "paused" : "running");
  })();
});

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({ id: MENU, title: t("menuDownloadLink"), contexts: ["link"] });
});

api.contextMenus.onClicked.addListener((/** @type {any} */ info) => {
  if (info.menuItemId !== MENU) return;
  void (async () => {
    const message = menuMessage(info);
    if (!message) {
      await notify(t("menuNotALink"));
      return;
    }
    await report(await send(api, message), "menu");
  })();
});

api.action.onClicked.addListener(() => {
  void api.runtime.openOptionsPage();
});
