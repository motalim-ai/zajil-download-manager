// @ts-check
// The HLS and DASH playlists a tab loads (M6, owner's decision): a page's
// player often plays a `blob:` the page made, and the playlist behind it is
// what Zajil can download. Only with "Allow Zajil on sites"; the requests
// are watched, never blocked or changed.
//
// What is noted stays in memory (`storage.session`, never on disk), for its
// tab only: forgotten when the tab loads another page or closes. It goes to
// Zajil only when the user asks for that tab's video. Nothing is noted from
// a private window.

import { isWeb } from "./capture.js";

/** At most this many playlists are kept for a tab (the newest). */
export const MAX_STREAMS = 20;

/** @typedef {{ url: string, kind: "hls" | "dash", frame: number }} Stream */

const HLS_TYPES = ["application/vnd.apple.mpegurl", "application/x-mpegurl", "audio/mpegurl", "audio/x-mpegurl"];
const DASH_TYPES = ["application/dash+xml"];

/** @param {number} tab */
const key = (tab) => `streams:${tab}`;

/**
 * What a response is: an HLS or a DASH playlist, by its `Content-Type`,
 * else by its path's extension (a server that says `text/plain`); or null.
 *
 * @param {{ url: string, responseHeaders?: { name: string, value?: string }[] }} details
 * @returns {"hls" | "dash" | null}
 */
export function kindOf(details) {
  const header = details.responseHeaders?.find((h) => h.name.toLowerCase() === "content-type")?.value ?? "";
  const type = header.split(";")[0].trim().toLowerCase();
  if (HLS_TYPES.includes(type)) return "hls";
  if (DASH_TYPES.includes(type)) return "dash";
  let path;
  try {
    path = new URL(details.url).pathname.toLowerCase();
  } catch {
    return null;
  }
  if (path.endsWith(".m3u8")) return "hls";
  if (path.endsWith(".mpd")) return "dash";
  return null;
}

/** Every change, one after the other: two playlists at once would each
 * read the old list and write it back without the other.
 * @type {Promise<unknown>} */
let changing = Promise.resolve();

/** @template T @param {() => Promise<T>} f @returns {Promise<T>} */
function inTurn(f) {
  const next = changing.then(f, f);
  changing = next.catch(() => {});
  return next;
}

/** @param {any} browserApi @param {number} tab @returns {Promise<Stream[]>} */
export async function streamsOf(browserApi, tab) {
  const found = (await browserApi.storage.session.get(key(tab)).catch(() => ({})))[key(tab)];
  return Array.isArray(found) ? found : [];
}

/** Forgets a tab's playlists. @param {any} browserApi @param {number} tab */
export async function forget(browserApi, tab) {
  await inTurn(() => browserApi.storage.session.remove(key(tab)).catch(() => {}));
}

/** Forgets every tab's playlists: the permission was taken back, so what
 * was noted under it goes too (review: a tab could otherwise send another
 * site's playlist with its next page).
 * @param {any} browserApi */
export async function forgetAll(browserApi) {
  await inTurn(async () => {
    const all = await browserApi.storage.session.get(null).catch(() => ({}));
    const keys = Object.keys(all ?? {}).filter((k) => k.startsWith("streams:"));
    if (keys.length) await browserApi.storage.session.remove(keys).catch(() => {});
  });
}

/**
 * Notes a response, if it is a playlist (`webRequest.onHeadersReceived`,
 * without blocking). A tab's new page forgets what its old one loaded. A
 * private window's tab is never noted: Firefox says so on the request;
 * Chrome is asked for the tab.
 *
 * @param {any} browserApi
 * @param {{ url: string, tabId: number, frameId?: number, type?: string, incognito?: boolean, responseHeaders?: { name: string, value?: string }[] }} details
 */
export async function note(browserApi, details) {
  const tab = details.tabId;
  if (!Number.isSafeInteger(tab) || tab < 0) return;
  if (details.type === "main_frame") {
    await forget(browserApi, tab);
    return;
  }
  const kind = kindOf(details);
  if (!kind || !isWeb(details.url) || details.incognito) return;
  const frame = Number.isSafeInteger(details.frameId) ? /** @type {number} */ (details.frameId) : 0;
  // In turn from here, Chrome's question included: a playlist of the old
  // page whose answer comes late is never written after the new page's
  // `main_frame` forgot the old ones.
  await inTurn(async () => {
    if (details.incognito === undefined) {
      const info = await browserApi.tabs.get(tab).catch(() => null);
      if (!info || info.incognito) return;
    }
    const kept = (await streamsOf(browserApi, tab)).filter((s) => s.url !== details.url);
    kept.push({ url: details.url, kind, frame });
    await browserApi.storage.session.set({ [key(tab)]: kept.slice(-MAX_STREAMS) }).catch(() => {});
  });
}

/**
 * The playlist to send with a tab's video whose own address only the page
 * can read: the newest of its frame, else the newest of the tab; or none.
 *
 * @param {any} browserApi @param {number} tab @param {number} [frame]
 * @returns {Promise<string | undefined>}
 */
export async function streamFor(browserApi, tab, frame = 0) {
  const all = await streamsOf(browserApi, tab);
  const own = all.filter((s) => s.frame === frame);
  return (own.at(-1) ?? all.at(-1))?.url;
}
