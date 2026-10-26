// @ts-check
// The download link's cookies, for Zajil (owner's decision on #7): only with
// the user's permission, asked for once with a button (the welcome page and
// the settings), never at install; only the cookies of that link; sent to
// Zajil on the same computer, which keeps them in memory only.
//
// The same button allows the video features (M6, owner's decision): the
// "Download video" button over a page's videos, and noting the HLS and DASH
// playlists a tab loads. One permission for the user: "Allow Zajil on
// sites".

/** The cookie permissions: the cookies API, and every site's cookies.
 * Nothing else (no history, no page content). */
export const COOKIE_PERMISSIONS = { permissions: ["cookies"], origins: ["<all_urls>"] };

/** The video permissions: the button over a page's videos (a script
 * registered on every site), and watching, never blocking, a tab's
 * requests for its HLS and DASH playlists. */
export const VIDEO_PERMISSIONS = { permissions: ["scripting", "webRequest"], origins: ["<all_urls>"] };

/** "Allow Zajil on sites": both, asked for and taken back together. */
export const SITE_PERMISSIONS = {
  permissions: [...COOKIE_PERMISSIONS.permissions, ...VIDEO_PERMISSIONS.permissions],
  origins: ["<all_urls>"],
};

/** @param {any} browserApi @param {{ permissions: string[], origins: string[] }} wanted */
async function has(browserApi, wanted) {
  try {
    return Boolean(await browserApi.permissions.contains(wanted));
  } catch {
    return false;
  }
}

/** Whether the user has given the cookie permission (also given before
 * the video features came, without them).
 * @param {any} browserApi */
export const cookiesAllowed = (browserApi) => has(browserApi, COOKIE_PERMISSIONS);

/** Whether the video features may run.
 * @param {any} browserApi */
export const videoAllowed = (browserApi) => has(browserApi, VIDEO_PERMISSIONS);

/** Whether "Allow Zajil on sites" is given, all of it.
 * @param {any} browserApi */
export const sitesAllowed = (browserApi) => has(browserApi, SITE_PERMISSIONS);

/**
 * What the welcome page and the settings show: the state's text, and which
 * buttons. "Partly" when only some of it is given (the cookies alone,
 * given before the video features came): it says what still goes, and
 * offers both to complete it and to take it back (review: it must never
 * say "not allowed" while cookies go).
 *
 * @param {any} browserApi
 * @returns {Promise<{ text: "cookiesAllowed" | "sitesPartly" | "cookiesNotAllowed", allow: boolean, drop: boolean }>}
 */
export async function permissionView(browserApi) {
  const [cookies, video] = [await cookiesAllowed(browserApi), await videoAllowed(browserApi)];
  if (cookies && video) return { text: "cookiesAllowed", allow: false, drop: true };
  if (cookies || video) return { text: "sitesPartly", allow: true, drop: true };
  return { text: "cookiesNotAllowed", allow: true, drop: false };
}

/** Asks for "Allow Zajil on sites". Must run from a user's click (Chrome).
 * @param {any} browserApi */
export async function askForSites(browserApi) {
  try {
    return Boolean(await browserApi.permissions.request(SITE_PERMISSIONS));
  } catch {
    return false;
  }
}

/** Gives "Allow Zajil on sites" back, all of it.
 * @param {any} browserApi */
export async function dropSites(browserApi) {
  try {
    await browserApi.permissions.remove(SITE_PERMISSIONS);
  } catch {
    // already gone
  }
}

/** The largest `Cookie` header `zajil-host` accepts (`MAX_COOKIES`). */
export const MAX_COOKIES = 16 * 1024;

/**
 * The `Cookie` header the browser would send to `url` from the cookie
 * store `storeId` (a Firefox container's, given by the download or the
 * tab; the default store when none), or null: no permission, no cookies, or
 * a header `zajil-host` would refuse (over 16 KiB, or anything but visible
 * ASCII and spaces: a line break would add a header). Then the download
 * goes without cookies, as without permission.
 *
 * For a link clicked with Zajil's keys, `from` is the page it was clicked
 * in, and its `SameSite=Strict` cookies go only if that page is of the
 * same site, as the browser does (owner's decision; `strictGoes`).
 * `noStrict`: none of them (a download asked for again whose link came
 * from another site).
 *
 * @param {any} browserApi
 * @param {string} url
 * @param {string} [storeId]
 * @param {{ from?: string, noStrict?: boolean }} [how]
 * @returns {Promise<string | null>}
 */
export async function cookieHeader(browserApi, url, storeId, how = {}) {
  if (!(await cookiesAllowed(browserApi))) return null;
  let cookies;
  try {
    // The cookies a request to `url` carries: its domain, path and
    // `Secure` rules applied by the browser itself, from the identity the
    // download came from (review of #76: a container's, not the default).
    cookies = await browserApi.cookies.getAll(storeId ? { url, storeId } : { url });
  } catch {
    // Firefox with first-party isolation asks for more; go without.
    return null;
  }
  const header = (cookies ?? [])
    .filter((/** @type {any} */ c) => typeof c?.name === "string" && typeof c?.value === "string")
    .filter((/** @type {any} */ c) => c.sameSite !== "strict" || (!how.noStrict && (how.from === undefined || strictGoes(how.from, url, c))))
    .map((/** @type {any} */ c) => (c.name ? `${c.name}=${c.value}` : c.value))
    .join("; ");
  if (!header || header.length > MAX_COOKIES || !/^[\x20-\x7e]+$/.test(header)) return null;
  return header;
}

/**
 * Whether a `SameSite=Strict` cookie of `url` goes with a link clicked in
 * the page `from`: only when they are of the same site. Without the list
 * of public suffixes, "same site" is proved by the cookie itself: `from`
 * has the same scheme and its host is the cookie's domain or under it (a
 * browser never sets a cookie on a public suffix, and `url`'s host is under
 * it too, or the browser would not have given the cookie). So it never
 * sends one the browser would not; it may keep back, rarely, one the
 * browser would send (a host-only cookie of `files.example.com` from a
 * page of `example.com`).
 *
 * @param {string} from @param {string} url @param {{ domain?: string }} cookie
 */
export function strictGoes(from, url, cookie) {
  let page;
  let link;
  try {
    [page, link] = [new URL(from), new URL(url)];
  } catch {
    return false;
  }
  if (page.protocol !== link.protocol) return false;
  const domain = String(cookie.domain ?? "").replace(/^\./, "").toLowerCase();
  const host = page.hostname.toLowerCase();
  return domain !== "" && (host === domain || host.endsWith(`.${domain}`));
}

/** Whether a link clicked in `from` goes to another host or scheme: then
 * a download asked for again later gets no strict cookie (`noStrict`).
 * @param {string} from @param {string} url */
export function otherSite(from, url) {
  try {
    const [page, link] = [new URL(from), new URL(url)];
    return page.protocol !== link.protocol || page.hostname !== link.hostname;
  } catch {
    return true;
  }
}
