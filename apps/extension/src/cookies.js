// @ts-check
// The download link's cookies, for Zajil (owner's decision on #7): only with
// the user's permission, asked for once with a button (the welcome page and
// the settings), never at install; only the cookies of that link; sent to
// Zajil on the same computer, which keeps them in memory only.

/** The optional permissions: the cookies API, and every site's cookies.
 * Nothing else (no history, no page content). */
export const COOKIE_PERMISSIONS = { permissions: ["cookies"], origins: ["<all_urls>"] };

/** The largest `Cookie` header `zajil-host` accepts (`MAX_COOKIES`). */
export const MAX_COOKIES = 16 * 1024;

/** Whether the user has given the cookie permission.
 * @param {any} browserApi */
export async function cookiesAllowed(browserApi) {
  try {
    return Boolean(await browserApi.permissions.contains(COOKIE_PERMISSIONS));
  } catch {
    return false;
  }
}

/** Asks for the cookie permission. Must run from a user's click (Chrome).
 * @param {any} browserApi */
export async function askForCookies(browserApi) {
  try {
    return Boolean(await browserApi.permissions.request(COOKIE_PERMISSIONS));
  } catch {
    return false;
  }
}

/** Gives the cookie permission back.
 * @param {any} browserApi */
export async function dropCookies(browserApi) {
  try {
    await browserApi.permissions.remove(COOKIE_PERMISSIONS);
  } catch {
    // already gone
  }
}

/**
 * The `Cookie` header the browser would send to `url` from the cookie
 * store `storeId` (a Firefox container's, given by the download or the
 * tab; the default store when none), or null: no permission, no cookies, or
 * a header `zajil-host` would refuse (over 16 KiB, or anything but visible
 * ASCII and spaces: a line break would add a header). Then the download
 * goes without cookies, as without permission.
 *
 * @param {any} browserApi
 * @param {string} url
 * @param {string} [storeId]
 * @returns {Promise<string | null>}
 */
export async function cookieHeader(browserApi, url, storeId) {
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
    .map((/** @type {any} */ c) => (c.name ? `${c.name}=${c.value}` : c.value))
    .join("; ");
  if (!header || header.length > MAX_COOKIES || !/^[\x20-\x7e]+$/.test(header)) return null;
  return header;
}
