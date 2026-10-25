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
 * most). Cookies are not sent yet (a later part of M5).
 *
 * @param {string} url
 * @param {{ referrer?: string, filename?: string }} [extra]
 */
export function downloadMessage(url, extra = {}) {
  /** @type {Record<string, unknown>} */
  const message = { type: "download", protocol_version: PROTOCOL_VERSION, url };
  if (extra.referrer && isWeb(extra.referrer)) message.referrer = extra.referrer;
  const name = extra.filename ? baseName(extra.filename) : "";
  // eslint-disable-next-line no-control-regex
  const plain = name && !/[\u0000-\u001f\u007f-\u009f]/.test(name) && name !== "." && name !== "..";
  if (plain && new TextEncoder().encode(name).length <= MAX_NAME) message.filename = name;
  return message;
}

/**
 * The message for "Download with Zajil" on a link, or null when the link is
 * not one Zajil can download (`javascript:`, `data:`, a password in it…).
 *
 * @param {{ linkUrl?: string, pageUrl?: string }} info
 */
export function menuMessage(info) {
  return isWeb(info.linkUrl) ? downloadMessage(/** @type {string} */ (info.linkUrl), { referrer: info.pageUrl }) : null;
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
 * @param {{ pause: boolean }} how `pause`: the browser keeps a paused
 *   download's data (`compat.pausesSafely`)
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
    downloadMessage(item.finalUrl || item.url, { referrer: item.referrer, filename: item.filename }),
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
