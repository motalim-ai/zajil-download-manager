// @ts-check
// Zajil's browser extension (M5, ADR-0005): downloads of the listed types go
// to Zajil through `zajil-host`, and "Download with Zajil" in a link's menu.
// Every listener is added at once, at the top: a Manifest V3 background
// stops when idle and is woken by these events.

import { api, onNewDownload } from "./compat.js";
import { downloadMessage, send, shouldCapture, takeOver } from "./capture.js";
import { load } from "./settings.js";

const MENU = "zajil-download-link";

/** @param {string} key */
const t = (key) => api.i18n.getMessage(key) || key;

/**
 * Tells the user what happened when Zajil did not take a download.
 * @param {import("./capture.js").Result | null} result
 */
async function report(result) {
  if (!result || result.outcome === "taken") {
    await api.action.setBadgeText({ text: "" });
    return;
  }
  const key = {
    refused: "notRefused",
    unknown: "notAnswered",
    unreachable: "notReachable",
  }[result.outcome];
  await api.action.setBadgeText({ text: result.outcome === "unknown" ? "?" : "!" });
  await api.notifications.create({
    type: "basic",
    iconUrl: "icons/128.png",
    title: t("extensionName"),
    message: t(key),
  });
}

onNewDownload(api, (item) => {
  void (async () => {
    const settings = await load(api);
    if (!shouldCapture(item, settings, api.runtime.id)) return;
    await report(await takeOver(api, item));
  })();
});

api.runtime.onInstalled.addListener(() => {
  api.contextMenus.create({ id: MENU, title: t("menuDownloadLink"), contexts: ["link"] });
});

api.contextMenus.onClicked.addListener((/** @type {any} */ info) => {
  if (info.menuItemId !== MENU || !info.linkUrl) return;
  void (async () => {
    await report(await send(api, downloadMessage(info.linkUrl, { referrer: info.pageUrl })));
  })();
});

api.action.onClicked.addListener(() => {
  void api.runtime.openOptionsPage();
});
