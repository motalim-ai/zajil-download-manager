// @ts-check
// The small compatibility layer between Chrome (and Edge) and Firefox
// (ADR-0005): one `api` object, and one way to hear of a new download.

/** The extension API: `browser` in Firefox, `chrome` in Chrome and Edge. Both
 * return promises in Manifest V3 for what this extension uses. */
export const api = /** @type {any} */ (globalThis).browser ?? /** @type {any} */ (globalThis).chrome;

/**
 * Whether pausing a download keeps what was downloaded, so it can be
 * resumed: yes in Chrome and Edge. In Firefox `downloads.pause` cancels a
 * download that has no data yet, and `resume` then fails (review of #70);
 * Chrome is told apart by `onDeterminingFilename`, which only it has.
 *
 * @param {any} browserApi
 */
export function pausesSafely(browserApi) {
  return Boolean(browserApi.downloads.onDeterminingFilename);
}

/**
 * Calls `handler` once for each new download, with its name when the browser
 * knows it. Chrome tells the name in `onDeterminingFilename` (Chrome only):
 * the browser's own name is kept at once (`suggest()`), and the handler runs
 * after. Firefox has no such event; its `onCreated` already has the name.
 *
 * @param {any} browserApi
 * @param {(item: any) => void} handler
 */
export function onNewDownload(browserApi, handler) {
  const downloads = browserApi.downloads;
  if (downloads.onDeterminingFilename) {
    downloads.onDeterminingFilename.addListener((/** @type {any} */ item, /** @type {() => void} */ suggest) => {
      suggest();
      handler(item);
    });
  } else {
    downloads.onCreated.addListener(handler);
  }
}

/** How many downloads are being handed to Zajil now, with the browser's own
 * download UI hidden (`withDownloadUiHidden`). */
let handingOver = 0;

/**
 * Runs `work` with the browser's own download UI (the bubble or bar)
 * hidden, so a download Zajil takes does not show in the browser
 * (`downloads.setUiOptions`, Chrome and Edge, with the `downloads.ui`
 * permission). It comes back once no hand-over is left, even when `work`
 * fails. Firefox has no such call: there `work` just runs.
 *
 * @template T
 * @param {any} browserApi
 * @param {() => Promise<T>} work
 * @returns {Promise<T>}
 */
export async function withDownloadUiHidden(browserApi, work) {
  const downloads = browserApi.downloads;
  if (typeof downloads.setUiOptions !== "function") return work();
  if (handingOver++ === 0) await downloads.setUiOptions({ enabled: false }).catch(() => {});
  try {
    return await work();
  } finally {
    if (--handingOver === 0) await downloads.setUiOptions({ enabled: true }).catch(() => {});
  }
}
