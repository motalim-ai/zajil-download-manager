// @ts-check
// Which downloads go to Zajil, the message `zajil-host` accepts, and handing a
// download over without losing it (ADR-0005).

/** The Native Messaging host's name (`zajil_platform::ids::NATIVE_HOST_NAME`). */
export const HOST = "io.github.motalim_ai.zajil.host";
/** `zajil_host::PROTOCOL_VERSION`. */
export const PROTOCOL_VERSION = 1;
/** The longest URL the host accepts (`zajil_host::MAX_URL`). */
export const MAX_URL = 8 * 1024;
/** The longest file name the host accepts, in UTF-8 bytes (`MAX_NAME`). */
export const MAX_NAME = 255;
/** How the host says the request reached Zajil but no answer came back (too
 * late, or broken): Zajil may have the download, so it is neither taken nor
 * refused (review of #62, #64, #70). */
export const NO_ANSWER = ["Zajil did not answer", "Zajil's answer is not valid"];

/** Control characters, C0 and C1, and the direction marks `zajil-host`
 * refuses in a URL (`queue::check_url`). */
// eslint-disable-next-line no-control-regex
const UNSAFE = /[\u0000-\u001f\u007f-\u009f\u061c\u200e\u200f\u2028\u2029\u202a-\u202e\u2066-\u2069]/;

/** A link the host would accept: http or https, no user name or password,
 * no control or direction characters, 8 KiB at most.
 * @param {string | undefined} url */
export function isWeb(url) {
  if (!url || url.length > MAX_URL || UNSAFE.test(url)) return false;
  try {
    const { protocol, username, password } = new URL(url);
    return (protocol === "http:" || protocol === "https:") && !username && !password;
  } catch {
    return false;
  }
}

/** The last part of a path, with `/` or `\` (Windows) between folders.
 * @param {string} path */
function baseName(path) {
  return path.split(/[\\/]/).pop() ?? "";
}

/** The file's extension, lower case: from the name the browser gave, else
 * from the URL's path.
 * @param {{ filename?: string, url?: string, finalUrl?: string }} item */
export function extensionOf(item) {
  let name = item.filename ? baseName(item.filename) : "";
  if (!name) {
    try {
      name = decodeURIComponent(baseName(new URL(item.finalUrl || item.url || "").pathname));
    } catch {
      name = "";
    }
  }
  const dot = name.lastIndexOf(".");
  return dot > 0 ? name.slice(dot + 1).toLowerCase() : "";
}

/** Whether `host` is `site` or one of its subdomains.
 * @param {string} host @param {string} site */
function onSite(host, site) {
  return host === site || host.endsWith(`.${site}`);
}

/**
 * Whether a new browser download goes to Zajil: capture is on, it is not a
 * private window's, it is an http(s) download the host would accept, of a
 * listed type, and neither its first URL, its final URL (after redirects)
 * nor the page it came from is on an excluded site.
 *
 * @param {any} item a `downloads.DownloadItem`
 * @param {import("./settings.js").Settings} settings
 * @param {string} [ownId] this extension's id: its own downloads stay
 */
export function shouldCapture(item, settings, ownId) {
  if (!settings.capture) return false;
  if (item.incognito) return false;
  if (item.state && item.state !== "in_progress") return false;
  if (ownId && item.byExtensionId === ownId) return false;
  const url = item.finalUrl || item.url;
  if (!isWeb(url)) return false;
  if (!settings.extensions.includes(extensionOf(item))) return false;
  for (const page of [item.url, item.finalUrl, item.referrer]) {
    if (!page) continue;
    let host;
    try {
      host = new URL(page).hostname;
    } catch {
      continue;
    }
    if (settings.excludedSites.some((site) => onSite(host, site))) return false;
  }
  return true;
}

/**
 * The `download` message for `zajil-host`: the URL, and the page and the
 * browser's file name when the host would accept them (a referrer that is
 * an http(s) URL; a plain name, no path, no control characters, 255 bytes at
 * most), and the link's `Cookie` header when the user allowed it
 * (`cookies.cookieHeader`, already checked as the host checks it). And
 * this extension's instance id and the cookie store (a Firefox container),
 * so Zajil can ask it for the cookies again after a restart (M5).
 *
 * @param {string} url
 * @param {{ referrer?: string, filename?: string, cookies?: string | null, instance?: string, store?: string, ticket?: string }} [extra]
 */
export function downloadMessage(url, extra = {}) {
  /** @type {Record<string, unknown>} */
  const message = { type: "download", protocol_version: PROTOCOL_VERSION, url };
  if (extra.referrer && isWeb(extra.referrer)) message.referrer = extra.referrer;
  const name = extra.filename ? baseName(extra.filename) : "";
  // eslint-disable-next-line no-control-regex
  const plain = name && !/[\u0000-\u001f\u007f-\u009f]/.test(name) && name !== "." && name !== "..";
  if (plain && new TextEncoder().encode(name).length <= MAX_NAME) message.filename = name;
  if (extra.cookies) message.cookies = extra.cookies;
  const id = /^[A-Za-z0-9_-]{1,64}$/;
  if (extra.instance && id.test(extra.instance)) message.instance = extra.instance;
  if (extra.store && id.test(extra.store) && extra.store !== "firefox-private") message.store = extra.store;
  if (extra.ticket && id.test(extra.ticket)) message.ticket = extra.ticket;
  return message;
}

/**
 * The message for "Download with Zajil" on a link, or null when the link is
 * not one Zajil can download (`javascript:`, `data:`, a password in it…).
 *
 * @param {{ linkUrl?: string, pageUrl?: string }} info
 * @param {string | null} [cookies] the link's `Cookie` header, if allowed
 * @param {{ instance?: string, store?: string, ticket?: string }} [from] where it comes from
 */
export function menuMessage(info, cookies = null, from = {}) {
  return isWeb(info.linkUrl)
    ? downloadMessage(/** @type {string} */ (info.linkUrl), { referrer: info.pageUrl, cookies, ...from })
    : null;
}

/**
 * The message for "Download video with Zajil" on a video or audio element
 * (M6, ADR-0009), or null when its page is not one Zajil can read. The
 * page goes to yt-dlp: that of the frame the video is in when it is in
 * one (an embedded player), else the tab's. The video's own address goes
 * too when it is http(s), for a page yt-dlp does not know; a `blob:` or
 * `data:` one, which only the page can read, does not.
 *
 * Without an address of its own, the playlist the tab loaded for it goes
 * instead (`streams.js`), when there is one.
 *
 * @param {{ pageUrl?: string, frameUrl?: string, srcUrl?: string }} info
 * @param {{ cookies?: string | null, private?: boolean, stream?: string }} [extra]
 *   the page's `Cookie` header, if allowed; whether it is a private
 *   window's; the playlist noted for it
 */
export function videoMessage(info, extra = {}) {
  const page = isWeb(info.frameUrl) ? info.frameUrl : info.pageUrl;
  if (!isWeb(page)) return null;
  /** @type {Record<string, unknown>} */
  const message = { type: "video", protocol_version: PROTOCOL_VERSION, page };
  if (isWeb(info.srcUrl)) message.media = info.srcUrl;
  else if (!extra.private && isWeb(extra.stream)) message.media = extra.stream;
  // A private window's cookies never go (as for a link).
  if (extra.private) message.private = true;
  else if (extra.cookies) message.cookies = extra.cookies;
  return message;
}

/** Whether Zajil refused because the site wants a sign-in: the server
 * answered 401 or 403 to Zajil (`zajil_core::Error::Status`).
 * @param {Result | null} result */
export function needsSignIn(result) {
  return result?.outcome === "refused" && /the server answered 40[13]\b/.test(result.error ?? "");
}

/**
 * What became of a request: `taken` (Zajil has it, and reached the file),
 * `refused` (it has not), `unknown` (it may have it: no answer came back),
 * or `unreachable` (no host: Zajil is not installed, or the browser
 * integration needs fixing).
 *
 * `alsoInBrowser`: taken, but the browser had finished it meanwhile
 * (Firefox, a small file): it is kept in the browser's list too.
 *
 * @typedef {{ outcome: "taken" | "refused" | "unknown" | "unreachable", error?: string, alsoInBrowser?: boolean }} Result
 */

/**
 * Sends `message` to Zajil through the host.
 * @param {any} browserApi
 * @param {Record<string, unknown>} message
 * @returns {Promise<Result>}
 */
export async function send(browserApi, message) {
  let answer;
  try {
    answer = await browserApi.runtime.sendNativeMessage(HOST, message);
  } catch (err) {
    return { outcome: "unreachable", error: String(err) };
  }
  if (answer && answer.ok === true) return { outcome: "taken" };
  const error = typeof answer?.error === "string" ? answer.error : "no answer";
  return { outcome: NO_ANSWER.some((start) => error.startsWith(start)) ? "unknown" : "refused", error };
}

/**
 * Hands a browser download to Zajil without losing it.
 *
 * - Where pausing keeps what was downloaded (Chrome and Edge), the browser's
 *   download is paused first. Taken by Zajil, it is cancelled and removed
 *   from the browser's list; refused or unreachable, it goes on in the
 *   browser; with no answer, it stays paused, since Zajil may have it.
 * - In Firefox, pausing a download that has no data yet cancels it for good
 *   (review of #70), so it is not paused: it goes on in the browser while
 *   Zajil is asked, and is cancelled only once Zajil has taken it.
 *
 * @param {any} browserApi
 * @param {any} item
 * @param {{ pause: boolean, cookies?: string | null, instance?: string, ticket?: string }} how
 *   `pause`: the browser keeps a paused download's data
 *   (`compat.pausesSafely`); `cookies`: the link's `Cookie` header, if the
 *   user allowed it; `instance`: this extension's id (`link.instanceId`)
 * @returns {Promise<Result | null>} null when the browser had already
 *   finished (nothing was sent)
 */
export async function takeOver(browserApi, item, how) {
  if (how.pause) {
    try {
      await browserApi.downloads.pause(item.id);
    } catch {
      return null;
    }
  }
  const result = await send(
    browserApi,
    downloadMessage(item.finalUrl || item.url, {
      referrer: item.referrer,
      filename: item.filename,
      cookies: how.cookies,
      instance: how.instance,
      store: item.cookieStoreId,
      ticket: how.ticket,
    }),
  );
  if (result.outcome === "taken") {
    // Not paused (Firefox), the browser may have finished it meanwhile:
    // then its file stays, and so does its line in the list (review of #70).
    if (!how.pause) {
      const [now] = await browserApi.downloads.search({ id: item.id }).catch(() => []);
      if (now?.state === "complete") return { ...result, alsoInBrowser: true };
    }
    await browserApi.downloads.cancel(item.id).catch(() => {});
    await browserApi.downloads.erase({ id: item.id }).catch(() => {});
  } else if (how.pause && result.outcome !== "unknown") {
    await browserApi.downloads.resume(item.id).catch(() => {});
  }
  return result;
}
