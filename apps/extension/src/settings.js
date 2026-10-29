// @ts-check
// What the extension captures, and the keys of a click, kept in
// `storage.local`.

/** The file types captured by default: Zajil's built-in categories
 * (`zajil_core::category::CategoriesFile::built_in`). */
export const DEFAULT_EXTENSIONS = [
  "exe", "msi", "msix", "msu", "dmg", "pkg", "deb", "rpm", "appimage", "apk", "flatpak", "iso", "img", "bin",
  "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "mpg", "mpeg",
  "3gp", "3g2", "rmvb", "rm", "ogv", "m2ts", "mts", "vob", "asf", "qt", "f4v",
  "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma", "aif", "aiff", "ra",
  "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", "txt", "rtf", "csv",
  "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "arj", "lzh", "cab",
  // The parts of a split RAR, R0* and R1*.
  "r00", "r01", "r02", "r03", "r04", "r05", "r06", "r07", "r08", "r09", "r10", "r11", "r12", "r13", "r14", "r15", "r16", "r17", "r18", "r19",
];

/** A file larger than this goes to Zajil whatever its type, by default
 * (owner's decision, 2026-10-27: on from the start, 5 MB). */
export const DEFAULT_BIG_FILE_MB = 5;
/** The largest size the setting takes, in MB. */
export const MAX_BIG_FILE_MB = 100000;

/**
 * @typedef {object} Settings
 * @property {boolean} capture Downloads of the listed types go to Zajil.
 * @property {string[]} extensions File types captured, lower case, no dot.
 * @property {boolean} bigFiles A file larger than `bigFileMb` goes to Zajil
 *   too, whatever its type, when the browser knows its size.
 * @property {number} bigFileMb That size, in MB of 1,048,576 bytes (as Zajil shows sizes), a whole
 *   number from 1 to `MAX_BIG_FILE_MB`.
 * @property {string[]} excludedSites Sites whose downloads stay in the
 *   browser: a host and its subdomains.
 * @property {boolean} useKeys The keys of a click are in use: off by
 *   default, as in other download managers; the user turns them on (owner's decision).
 * @property {string | null} sendKeys The keys that, held while clicking a
 *   link, send it to Zajil (one of `KEY_CHOICES`); null: this system's
 *   default (`defaultKeys`).
 * @property {string | null} browserKeys The keys that leave a clicked
 *   link to the browser, which then downloads it without Zajil taking it;
 *   null: this system's default.
 */

/** The keys a click can be held with (owner's decision, 2026-10-26): the
 * modifiers in this order, joined by `+`; `off` for none. Never Cmd or the
 * Windows key, nor Shift alone (a new window). The same list is in
 * `click-keys.js`, which cannot import this module. */
export const KEY_CHOICES = ["off", "ctrl", "alt", "ctrl+shift", "alt+shift", "ctrl+alt"];

/** Whether this is a Mac, where Option (`alt`) is the key and Control+click
 * opens the menu. @param {{ platform?: string, userAgentData?: { platform?: string } }} nav */
export function isMac(nav) {
  return /^mac/i.test(nav.userAgentData?.platform || nav.platform || "");
}

/** The owner's defaults: Ctrl+click sends to Zajil and Alt+click leaves it
 * to the browser on Windows and Linux; on a Mac, Option+click and
 * Option+Shift+click. @param {boolean} mac */
export function defaultKeys(mac) {
  return mac ? { send: "alt", browser: "alt+shift" } : { send: "ctrl", browser: "alt" };
}

/** The keys in force: the stored ones, else this system's; the browser's
 * keys off when they are the same as Zajil's.
 * @param {Pick<Settings, "sendKeys" | "browserKeys">} settings @param {boolean} mac */
export function clickKeys(settings, mac) {
  const base = defaultKeys(mac);
  const send = settings.sendKeys ?? base.send;
  const browser = settings.browserKeys ?? base.browser;
  return { send, browser: browser === send ? "off" : browser };
}

/** @param {unknown} value @returns {number | null} */
const sizeOrNull = (value) => (Number.isInteger(value) && /** @type {number} */ (value) >= 1 && /** @type {number} */ (value) <= MAX_BIG_FILE_MB ? /** @type {number} */ (value) : null);

/** @param {unknown} value @returns {string | null} */
const keysOrNull = (value) => (typeof value === "string" && KEY_CHOICES.includes(value) ? value : null);

/** @returns {Settings} */
export function defaults() {
  return {
    capture: true,
    extensions: [...DEFAULT_EXTENSIONS],
    bigFiles: true,
    bigFileMb: DEFAULT_BIG_FILE_MB,
    excludedSites: [],
    useKeys: false,
    sendKeys: null,
    browserKeys: null,
  };
}

/**
 * The settings in storage, with defaults for what is missing or wrong.
 * @param {any} browserApi
 * @returns {Promise<Settings>}
 */
export async function load(browserApi) {
  const stored = (await browserApi.storage.local.get("settings")).settings ?? {};
  const base = defaults();
  return {
    capture: typeof stored.capture === "boolean" ? stored.capture : base.capture,
    extensions: Array.isArray(stored.extensions) ? normalizeExtensions(stored.extensions) : base.extensions,
    bigFiles: typeof stored.bigFiles === "boolean" ? stored.bigFiles : base.bigFiles,
    bigFileMb: sizeOrNull(stored.bigFileMb) ?? base.bigFileMb,
    excludedSites: Array.isArray(stored.excludedSites) ? normalizeSites(stored.excludedSites) : base.excludedSites,
    useKeys: stored.useKeys === true,
    sendKeys: keysOrNull(stored.sendKeys),
    browserKeys: keysOrNull(stored.browserKeys),
  };
}

/**
 * @param {any} browserApi
 * @param {Settings} settings
 */
export async function save(browserApi, settings) {
  await browserApi.storage.local.set({
    settings: {
      capture: settings.capture,
      extensions: normalizeExtensions(settings.extensions),
      bigFiles: settings.bigFiles === true,
      bigFileMb: sizeOrNull(settings.bigFileMb) ?? DEFAULT_BIG_FILE_MB,
      excludedSites: normalizeSites(settings.excludedSites),
      useKeys: settings.useKeys === true,
      sendKeys: keysOrNull(settings.sendKeys),
      browserKeys: keysOrNull(settings.browserKeys),
    },
  });
}

/** `.ZIP`, ` zip ` → `zip`; empty and odd entries dropped, no repeats.
 * @param {unknown[]} list */
export function normalizeExtensions(list) {
  const out = list
    .filter((x) => typeof x === "string")
    .map((x) => /** @type {string} */ (x).trim().replace(/^\./, "").toLowerCase())
    .filter((x) => /^[a-z0-9]{1,16}$/.test(x));
  return [...new Set(out)];
}

/** `https://Example.com/x`, `example.com.` → `example.com`; odd entries
 * dropped, no repeats.
 * @param {unknown[]} list */
export function normalizeSites(list) {
  const out = [];
  for (const entry of list) {
    if (typeof entry !== "string") continue;
    let text = entry.trim().toLowerCase();
    if (!text) continue;
    if (!text.includes("://")) text = `https://${text}`;
    try {
      const host = new URL(text).hostname.replace(/\.$/, "");
      if (host) out.push(host);
    } catch {
      // not a site: dropped
    }
  }
  return [...new Set(out)];
}
