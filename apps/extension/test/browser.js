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
