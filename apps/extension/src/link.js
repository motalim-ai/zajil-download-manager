// @ts-check
// The link to Zajil (M5, owner's decision on #7): after a restart Zajil asks
// this browser for the cookies of a download that came from it, while the
// browser is open. The extension keeps a port to `zajil-host` open
// (`runtime.connectNative`); the host passes Zajil's questions on it, one
// at a time, and the answers back.
//
// The extension answers only for a download it sent to Zajil with cookies
// itself and that is still open (owner's decision on #80): each such
// download has a random ticket, kept with a hash of its site and its cookie
// store (`storage.local`, never the cookies). Zajil asks by ticket, and
// tells the extension which tickets are still open (`keep`): the others,
// done or removed, are forgotten at once. 30 days is the most a ticket of
// a download that never ends is kept.

import { HOST, PROTOCOL_VERSION, isWeb } from "./capture.js";
import { cookieHeader } from "./cookies.js";

/** The most days a ticket is kept, for a download that never ends. */
export const TICKET_DAYS = 30;
/** At most this many tickets are kept (the oldest go first). */
export const MAX_TICKETS = 2000;
/** A ticket this young is kept even if a `keep` does not list it: Zajil
 * may have listed the open tickets just before this download reached it. */
export const NEW_TICKET_MS = 60_000;
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

/** Every change of the tickets, one after the other: two at once (two
 * downloads taken together, or a `keep` meanwhile) would each read the old
 * list and write it back without the other's ticket (review of #80).
 * @type {Promise<unknown>} */
let changing = Promise.resolve();

/** @template T @param {() => Promise<T>} f @returns {Promise<T>} */
function inTurn(f) {
  const next = changing.then(f, f);
  changing = next.catch(() => {});
  return next;
}

/** A new download's random ticket. */
export const newTicket = () => crypto.randomUUID();

/** @param {string} url @param {string | undefined} store */
async function siteKey(url, store) {
  const text = `${new URL(url).origin}|${store ?? ""}`;
  const digest = await crypto.subtle.digest("SHA-256", new TextEncoder().encode(text));
  return [...new Uint8Array(digest)].map((b) => b.toString(16).padStart(2, "0")).join("");
}

/** `noStrict`: its link came from another site's page with Zajil's keys,
 * so no `SameSite=Strict` cookie goes with it (`cookieHeader`).
 * @typedef {Record<string, { site: string, at: number, noStrict?: boolean }>} Tickets */

/** @param {any} browserApi @returns {Promise<Tickets>} */
async function tickets(browserApi) {
  const { linkTickets } = await browserApi.storage.local.get("linkTickets").catch(() => ({}));
  return linkTickets && typeof linkTickets === "object" ? linkTickets : {};
}

/** @param {Tickets} all @param {number} now */
function fresh(all, now) {
  const limit = now - TICKET_DAYS * 24 * 3600 * 1000;
  const kept = Object.entries(all).filter(([, t]) => typeof t?.at === "number" && t.at >= limit && typeof t.site === "string");
  kept.sort((a, b) => b[1].at - a[1].at);
  return Object.fromEntries(kept.slice(0, MAX_TICKETS));
}

/** Remembers that download `ticket` went to Zajil with the cookies of
 * `url`'s site from `store`: Zajil may ask for them again while it is open.
 * @param {any} browserApi @param {string} ticket @param {string} url
 * @param {string | undefined} store @param {number} [now]
 * @param {{ noStrict?: boolean }} [how] */
export async function rememberTicket(browserApi, ticket, url, store, now = Date.now(), how = {}) {
  const site = await siteKey(url, store);
  await inTurn(async () => {
    const all = { ...(await tickets(browserApi)), [ticket]: how.noStrict ? { site, at: now, noStrict: true } : { site, at: now } };
    await browserApi.storage.local.set({ linkTickets: fresh(all, now) }).catch(() => {});
  });
}

/** The addresses a download's link leads to that are another origin of
 * the same site, by the origin of the link: a Google Drive file's page is
 * downloaded from Drive's download address (owner's decision on the Drive
 * links: Zajil asks for that address's own cookies, never another site's). */
const SAME_SITE_DOWNLOADS = /** @type {Record<string, string[]>} */ ({
  "https://drive.usercontent.google.com": ["https://drive.google.com"],
});

/** Whether `ticket` is open, for `url`'s site and `store`: the download's
 * own origin, or its link's download address of the same site.
 * @param {any} browserApi @param {string} ticket @param {string} url
 * @param {string | undefined} store @param {number} [now] */
export async function openTicket(browserApi, ticket, url, store, now = Date.now()) {
  return Boolean(await ticketFor(browserApi, ticket, url, store, now));
}

/** The open `ticket` for `url`'s site and `store`, or null.
 * @param {any} browserApi @param {string} ticket @param {string} url
 * @param {string | undefined} store @param {number} [now] */
async function ticketFor(browserApi, ticket, url, store, now = Date.now()) {
  const found = fresh(await tickets(browserApi), now)[ticket];
  if (!found) return null;
  if (found.site === (await siteKey(url, store))) return found;
  for (const link of SAME_SITE_DOWNLOADS[new URL(url).origin] ?? []) {
    if (found.site === (await siteKey(link, store))) return found;
  }
  return null;
}

/** Forgets `ticket`: Zajil did not take its download.
 * @param {any} browserApi @param {string} ticket */
export async function forgetTicket(browserApi, ticket) {
  await inTurn(async () => {
    const all = await tickets(browserApi);
    if (!(ticket in all)) return;
    delete all[ticket];
    await browserApi.storage.local.set({ linkTickets: all }).catch(() => {});
  });
}

/** Keeps only the tickets Zajil says are open (and the newest ones).
 * @param {any} browserApi @param {string[]} open @param {number} [now] */
export async function keepTickets(browserApi, open, now = Date.now()) {
  const listed = new Set(open);
  await inTurn(async () => {
    const kept = Object.fromEntries(
      Object.entries(fresh(await tickets(browserApi), now)).filter(([id, t]) => listed.has(id) || now - t.at < NEW_TICKET_MS),
    );
    await browserApi.storage.local.set({ linkTickets: kept }).catch(() => {});
  });
}

/**
 * The answer to one of Zajil's messages on the link, or null for a message
 * that is not a question. The cookies are given only with the permission
 * (`cookieHeader`), and only for an open download's ticket whose site and
 * store are the ones asked about; else the answer has none, and Zajil goes
 * on without them. A `keep` is applied, and answered without cookies.
 *
 * @param {any} browserApi
 * @param {any} message
 * @returns {Promise<Record<string, unknown> | null>}
 */
export async function answer(browserApi, message) {
  if (!Number.isSafeInteger(message?.ask) || message.ask < 0) return null;
  const reply = (/** @type {string | null} */ cookies) => ({ type: "cookies", protocol_version: PROTOCOL_VERSION, ask: message.ask, cookies });
  if (message.type === "keep") {
    if (!Array.isArray(message.tickets) || !message.tickets.every((/** @type {unknown} */ t) => typeof t === "string")) return reply(null);
    await keepTickets(browserApi, message.tickets);
    return reply(null);
  }
  if (message.type !== "cookies_wanted") return null;
  const store = plainStore(message.store);
  const wrongStore = message.store !== undefined && message.store !== null && store === undefined;
  if (!isWeb(message.url) || wrongStore || typeof message.ticket !== "string" || !ID.test(message.ticket)) return reply(null);
  const found = await ticketFor(browserApi, message.ticket, message.url, store);
  if (!found) return reply(null);
  return reply(await cookieHeader(browserApi, message.url, store, { noStrict: found.noStrict === true }));
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
