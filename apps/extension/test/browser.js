// The real browser of the browser tests (#174): on CI the runner's Chrome,
// here Playwright's Chromium. Its start is timed and printed (when it began
// and how long it took, and every 2 s while it takes), so a slow start on
// CI shows its length, and whether the test files' browsers started at the
// same moment.
import { chromium } from "playwright-core";

// Past Vitest's console, which keeps a passing file's lines to itself.
/** @param {string} line */
const say = (line) => process.stderr.write(`${line}\n`);

/** How long a test file's `beforeAll` gives the browser to start: it took
 * from 0.2 s to 13.7 s on CI (#174), more than Vitest's 10 s. */
export const BROWSER_START = 30_000;

/** Starts the browser for test file `name`, saying how long it took.
 * @param {string} name */
export async function launch(name) {
  const start = performance.now();
  const ms = () => Math.round(performance.now() - start);
  say(`[${name}] the browser starts at ${new Date().toISOString()}`);
  const waiting = setInterval(() => say(`[${name}] the browser is still starting after ${ms()} ms`), 2000);
  try {
    const browser = await chromium.launch(process.env.CI ? { channel: "chrome" } : {});
    say(`[${name}] the browser started in ${ms()} ms, at ${new Date().toISOString()}`);
    return browser;
  } finally {
    clearInterval(waiting);
  }
}

/** Starts the browser once, and opens one page in it as the tests do,
 * before any test file starts its own (#230, #296). On CI the runner's
 * first Chrome is often slow, and so is its first page: from 5.4 s to
 * 27.7 s to start in 27 of 40 jobs (and over 30 s in 2 more), then about a
 * third of that for the first test of each file (over 5 s from a 14.3 s
 * start), while the tests after it soon take their usual times. The two
 * files' browsers finished starting within 5 ms of each other in 39 of the
 * 40 jobs, though they began up to 0.6 s apart, and even with every other
 * test file done: the runner held them, not the tests. Paid here once, no
 * test or `beforeAll` pays it. A browser that cannot start here is left to
 * each file's `beforeAll`, which says why. */
export async function warmUp() {
  const start = performance.now();
  let browser;
  try {
    browser = await launch("warm-up");
    const page = await browser.newPage({ viewport: { width: 800, height: 400 } });
    await page.route("**/*", (route) =>
      route.request().url() === "https://page.invalid/"
        ? route.fulfill({ contentType: "text/html", body: `<!doctype html><body><p id="p">Zajil</p><video src="https://media.invalid/v.mp4"></video></body>` })
        : route.abort(),
    );
    await page.goto("https://page.invalid/");
    await page.addScriptTag({ content: "document.body.dataset.ready = '1';" });
    await page.mouse.move(50, 50);
    await page.click("#p", { modifiers: ["Shift"] });
    say(`[warm-up] the browser and its first page took ${Math.round(performance.now() - start)} ms`);
  } catch (err) {
    say(`[warm-up] failed after ${Math.round(performance.now() - start)} ms: ${err}`);
  } finally {
    await browser?.close().catch(() => {});
  }
}
