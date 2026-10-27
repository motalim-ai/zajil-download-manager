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

/** The longest a new download waits for the extension to decide, in
 * Chrome and Edge, before the browser goes on with its own name. */
export const DECIDE_MS = 3000;

/**
 * Calls `handler` once for each new download, with its name when the browser
 * knows it, and `ready`, to call once it has decided (and, for a download
 * Zajil takes, hidden the browser's download UI).
 *
 * Chrome tells the name in `onDeterminingFilename` (Chrome only), and keeps
 * the download waiting until `suggest()`: the browser's own name is kept,
 * but only once the handler is ready, since Chrome decides whether to show
 * a download in its bubble when its name is settled, and never again (review
 * of #253: hidden after, the bubble still showed). It is called anyway when
 * the handler ends or fails, or after `DECIDE_MS`, so no download is held.
 * Firefox has no such event; its `onCreated` already has the name, and
 * `ready` does nothing.
 *
 * @param {any} browserApi
 * @param {(item: any, ready: () => void) => Promise<void>} handler
 */
export function onNewDownload(browserApi, handler) {
  const downloads = browserApi.downloads;
  if (downloads.onDeterminingFilename) {
    downloads.onDeterminingFilename.addListener((/** @type {any} */ item, /** @type {() => void} */ suggest) => {
      let done = false;
      const ready = () => {
        if (done) return;
        done = true;
        clearTimeout(late);
        try {
          suggest();
        } catch {
          // The download was already settled or gone.
        }
      };
      const late = setTimeout(ready, DECIDE_MS);
      void Promise.resolve()
        .then(() => handler(item, ready))
        .catch(() => {})
        .finally(ready);
      // `suggest` is called later (Chrome's way of saying so).
      return true;
    });
  } else {
    downloads.onCreated.addListener((/** @type {any} */ item) => {
      void handler(item, () => {}).catch(() => {});
    });
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
