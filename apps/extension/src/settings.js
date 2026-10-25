// @ts-check
// What the extension captures, kept in `storage.local`.

/** The file types captured by default: Zajil's built-in categories
 * (`zajil_core::category::CategoriesFile::built_in`). */
export const DEFAULT_EXTENSIONS = [
  "exe", "msi", "msix", "dmg", "pkg", "deb", "rpm", "appimage", "apk", "flatpak", "iso", "img",
  "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "mpg", "mpeg",
  "mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma",
  "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", "txt", "rtf", "csv",
  "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst",
];

/**
 * @typedef {object} Settings
 * @property {boolean} capture Downloads of the listed types go to Zajil.
 * @property {string[]} extensions File types captured, lower case, no dot.
 * @property {string[]} excludedSites Sites whose downloads stay in the
 *   browser: a host and its subdomains.
 */

/** @returns {Settings} */
export function defaults() {
  return { capture: true, extensions: [...DEFAULT_EXTENSIONS], excludedSites: [] };
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
    excludedSites: Array.isArray(stored.excludedSites) ? normalizeSites(stored.excludedSites) : base.excludedSites,
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
      excludedSites: normalizeSites(settings.excludedSites),
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
