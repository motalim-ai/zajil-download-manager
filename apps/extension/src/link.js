// @ts-check
// The link to Zajil (M5, owner's decision on #7): after a restart Zajil asks
// this browser for the cookies of a download that came from it, while the
// browser is open. The extension keeps a port to `zajil-host` open
// (`runtime.connectNative`); the host passes Zajil's questions on it, one
// at a time, and the answers back.
//
// The extension answers only for a site it sent to Zajil with cookies
// itself, in the same cookie store, in the last 30 days: anything else
// that opened Zajil's channel cannot ask it for other sites' cookies. Of
// those sites only a hash is kept (`storage.local`), never the cookies.

import { HOST, PROTOCOL_VERSION, isWeb } from "./capture.js";
import { cookieHeader } from "./cookies.js";

/** How long a site stays one Zajil may ask about. */
export const SITE_DAYS = 30;
/** At most this many sites are kept (the oldest go first). */
export const MAX_SITES = 500;
/** The longest wait between two tries to link (Zajil's host missing, or
 * the browser closed it). */
export const MAX_RETRY_MS = 60_000;

/** An instance or cookie store id, as `zajil-host` accepts it. */
const ID = /^[A-Za-z0-9_-]{1,64}$/;

/** This extension's own random id: one per browser profile, so Zajil asks
 * the same browser again. Made once, kept in `storage.local`.
 * @param {any} browserApi
 * @returns {Promise<string>} */
export async function instanceId(browserApi) {
  const { instance } = await browserApi.storage.local.get("instance").catch(() => ({}));
  if (typeof instance === "string" && ID.test(instance)) return instance;
  const made = crypto.randomUUID();
  await browserApi.storage.local.set({ instance: made }).catch(() => {});
  return made;
}

/** A cookie store id that may go to Zajil: none (the default store), or a
 * plain id; never a private window's.
 * @param {unknown} store */
export function plainStore(store) {
  return typeof store === "string" && ID.test(store) && store !== "firefox-private" ? store : undefined;
}

/** @param {string} url @param {string | undefined} store */
async function siteKey(url, store) {
  const text = `${new URL(url).origin}|${store ?? ""}`;
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** @param {Record<string, number>} sites @param {number} now */
function fresh(sites, now) {
  const limit = now - SITE_DAYS * 24 * 3600 * 1000;
  const kept = Object.entries(sites).filter(([, at]) => typeof at === "number" && at >= limit);
  kept.sort((a, b) => b[1] - a[1]);
  return Object.fromEntries(kept.slice(0, MAX_SITES));
}

/** Remembers that the cookies of `url`'s site (from `store`) went to
 * Zajil, so it may ask for them again after a restart.
 * @param {any} browserApi @param {string} url @param {string | undefined} store
 * @param {number} [now] */
export async function rememberSite(browserApi, url, store, now = Date.now()) {
  const key = await siteKey(url, store);
  const { linkSites } = await browserApi.storage.local.get("linkSites").catch(() => ({}));
  const sites = fresh({ ...(linkSites ?? {}), [key]: now }, now);
  await browserApi.storage.local.set({ linkSites: sites }).catch(() => {});
}

/** @param {any} browserApi @param {string} url @param {string | undefined} store
 * @param {number} [now] */
export async function knownSite(browserApi, url, store, now = Date.now()) {
  const { linkSites } = await browserApi.storage.local.get("linkSites").catch(() => ({}));
  return Object.hasOwn(fresh(linkSites ?? {}, now), await siteKey(url, store));
}

/**
 * The answer to one of Zajil's messages on the link, or null for a message
 * that is not a question. The cookies are given only with the permission
 * (`cookieHeader`), and only for a site this extension sent to Zajil with
 * cookies; else the answer has none, and Zajil goes on without them.
 *
 * @param {any} browserApi
 * @param {any} message
 * @returns {Promise<Record<string, unknown> | null>}
 */
export async function answer(browserApi, message) {
  if (message?.type !== "cookies_wanted" || !Number.isSafeInteger(message.ask) || message.ask < 0) return null;
  const store = plainStore(message.store);
  const wrongStore = message.store !== undefined && message.store !== null && store === undefined;
  let cookies = null;
  if (isWeb(message.url) && !wrongStore && (await knownSite(browserApi, message.url, store))) {
    cookies = await cookieHeader(browserApi, message.url, store);
  }
  return { type: "cookies", protocol_version: PROTOCOL_VERSION, ask: message.ask, cookies };
}

/**
 * Opens the link, and opens it again whenever it closes (Zajil's host
 * missing, the browser closed it), waiting longer each time, up to
 * MAX_RETRY_MS; a question answered shows the link works, and the wait
 * starts over.
 *
 * @param {any} browserApi
 * @param {{ wait?: (f: () => void, ms: number) => unknown }} [how]
 */
export function startLink(browserApi, how = {}) {
  const wait = how.wait ?? ((f, ms) => setTimeout(f, ms));
  let delay = 2000;
  const open = async () => {
    let port;
    try {
      port = browserApi.runtime.connectNative(HOST);
    } catch {
      wait(open, delay);
      delay = Math.min(delay * 2, MAX_RETRY_MS);
      return;
    }
    port.onMessage.addListener((/** @type {any} */ message) => {
      void answer(browserApi, message).then((reply) => {
        if (!reply) return;
        delay = 2000;
        try {
          port.postMessage(reply);
        } catch {
          // closed meanwhile: Zajil gets no answer, and asks again
        }
      });
    });
    port.onDisconnect.addListener(() => {
      wait(() => void open(), delay);
      delay = Math.min(delay * 2, MAX_RETRY_MS);
    });
    const instance = await instanceId(browserApi);
    try {
      port.postMessage({ type: "link", protocol_version: PROTOCOL_VERSION, instance });
    } catch {
      // closed meanwhile: onDisconnect opens it again
    }
  };
  void open();
}
