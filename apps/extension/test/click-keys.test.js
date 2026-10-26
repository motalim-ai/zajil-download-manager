// The keys of a click (`click-keys.js`) in a real browser: a link clicked
// with Zajil's keys goes to the background and nowhere else; with the
// browser's keys the browser keeps it; anything else is the page's own. On
// CI, the runner's Chrome; here, Playwright's Chromium. No site is reached.
import { readFileSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { BROWSER_START, launch } from "./browser.js";

const script = readFileSync(new URL("../src/click-keys.js", import.meta.url), "utf8");

/** @type {import("playwright-core").Browser} */
let browser;

// Starting the runner's Chrome took from 0.2 s to 13.7 s on CI (#174):
// 30 s, not Vitest's 10.
beforeAll(async () => {
  browser = await launch("click-keys");
}, BROWSER_START);

afterAll(async () => {
  await browser?.close();
});

const LINK = `<a id="a" href="https://files.invalid/a.zip" style="position:absolute;left:50px;top:50px;font-size:40px">a.zip</a>`;

/** A page with a link the page also listens to, and the script with a fake
 * `chrome` that records what is sent. `stored`: the extension's settings;
 * `mac`: the page says it is a Mac; `on`: the background's word that
 * "Allow Zajil on sites" is given (`clickKeysOn`).
 * @param {{ stored?: object, mac?: boolean, body?: string, on?: boolean }} [how] */
async function pageWith(how = {}) {
  const context = await browser.newContext({ viewport: { width: 800, height: 400 } });
  const page = await context.newPage();
  await page.route("**/*", (route) =>
    route.request().url() === "https://page.invalid/"
      ? route.fulfill({ contentType: "text/html", body: `<!doctype html><body>${how.body ?? LINK}</body>` })
      : route.abort(),
  );
  await page.addInitScript(
    ({ stored, mac, on }) => {
      const w = /** @type {any} */ (window);
      w.sent = [];
      w.pageSaw = [];
      document.addEventListener("click", (e) => w.pageSaw.push(e.defaultPrevented));
      Object.defineProperty(Navigator.prototype, "platform", { get: () => (mac ? "MacIntel" : "Linux x86_64") });
      Object.defineProperty(Navigator.prototype, "userAgentData", { get: () => undefined });
      w.chrome = {
        runtime: { id: "zajil", sendMessage: async (/** @type {unknown} */ m) => w.sent.push(m) },
        storage: {
          local: { get: async () => ({ settings: stored, clickKeysOn: on }) },
          onChanged: { addListener: (/** @type {any} */ fn) => (w.changed = fn) },
        },
      };
    },
    // The keys turned on in the settings, unless the test says otherwise.
    { stored: { useKeys: true, ...how.stored }, mac: how.mac ?? false, on: how.on ?? true },
  );
  await page.goto("https://page.invalid/");
  await page.addScriptTag({ content: script });
  await page.waitForTimeout(50);
  return { page, context };
}

const sent = (/** @type {import("playwright-core").Page} */ page) => page.evaluate(() => /** @type {any} */ (window).sent);
const pageSaw = (/** @type {import("playwright-core").Page} */ page) => page.evaluate(() => /** @type {any} */ (window).pageSaw);

/** Clicks the link with `modifiers`. @param {import("playwright-core").Page} page @param {any[]} modifiers */
async function click(page, modifiers) {
  await page.click("#a", { modifiers, noWaitAfter: true });
  await page.waitForTimeout(100);
}

/** On a Mac, Control+click is the right click: no click event to test. */
const ctrlClicks = process.platform !== "darwin";

describe("the keys of a click", () => {
  it.runIf(ctrlClicks)("Ctrl+click sends the link to Zajil, and the browser and the page do nothing with it (Windows and Linux)", async () => {
    const { page, context } = await pageWith();
    await click(page, ["Control"]);
    expect(await sent(page)).toEqual([{ type: "click-link", href: "https://files.invalid/a.zip" }]);
    // No new tab, and the page's own listener never heard of it.
    expect(context.pages()).toHaveLength(1);
    expect(await pageSaw(page)).toEqual([]);
    await context.close();
  });

  it("Alt+click leaves it to the browser and says so; a plain click and Shift+click are the page's own", async () => {
    const { page, context } = await pageWith();
    await click(page, ["Alt"]);
    expect(await sent(page)).toEqual([{ type: "click-browser", href: "https://files.invalid/a.zip" }]);
    expect(await pageSaw(page)).toEqual([false]);
    await click(page, ["Shift"]);
    await page.evaluate(() => document.getElementById("a")?.addEventListener("click", (e) => e.preventDefault()));
    await click(page, []);
    expect(await sent(page)).toHaveLength(1);
    await context.close();
  });

  it("on a Mac, Option+click sends it and Option+Shift+click leaves it to the browser; Cmd never counts", async () => {
    const { page, context } = await pageWith({ mac: true });
    await click(page, ["Alt"]);
    await click(page, ["Alt", "Shift"]);
    await click(page, ["Meta", "Alt"]);
    expect(await sent(page)).toEqual([
      { type: "click-link", href: "https://files.invalid/a.zip" },
      { type: "click-browser", href: "https://files.invalid/a.zip" },
    ]);
    await context.close();
  });

  it("takes the keys from the settings, and their changes at once", async () => {
    const { page, context } = await pageWith({ stored: { sendKeys: "alt+shift", browserKeys: "off" } });
    await click(page, ["Alt"]);
    expect(await sent(page)).toEqual([]);
    await click(page, ["Alt", "Shift"]);
    expect(await sent(page)).toEqual([{ type: "click-link", href: "https://files.invalid/a.zip" }]);
    await page.evaluate(() => /** @type {any} */ (window).changed({ settings: { newValue: { useKeys: true, sendKeys: "off", browserKeys: "alt" } } }, "local"));
    await click(page, ["Alt", "Shift"]);
    await click(page, ["Alt"]);
    expect((await sent(page)).at(-1)).toEqual({ type: "click-browser", href: "https://files.invalid/a.zip" });
    expect(await sent(page)).toHaveLength(2);
    await context.close();
  });

  it("a click the page makes itself, or on what is not a web link, sends nothing", async () => {
    const { page, context } = await pageWith({
      stored: { sendKeys: "alt" },
      body: `${LINK}<a id="j" href="javascript:void 0" style="position:absolute;left:50px;top:150px;font-size:40px">js</a>`,
    });
    // Made by the page (kept from navigating, as the test needs).
    await page.evaluate(() => {
      const a = /** @type {HTMLElement} */ (document.getElementById("a"));
      a.addEventListener("click", (e) => e.preventDefault());
      a.dispatchEvent(new MouseEvent("click", { bubbles: true, cancelable: true, altKey: true }));
    });
    await page.click("#j", { modifiers: ["Alt"], noWaitAfter: true });
    await page.waitForTimeout(100);
    expect(await sent(page)).toEqual([]);
    await context.close();
  });

  it("takes no click without the permission, nor once the extension is gone from an open tab (review of #169)", async () => {
    const { page, context } = await pageWith({ on: false, stored: { sendKeys: "alt" } });
    await click(page, ["Alt"]);
    expect(await sent(page)).toEqual([]);
    expect(await pageSaw(page)).toEqual([false]);
    // Given: at once. Taken back: at once, in this open tab.
    await page.evaluate(() => /** @type {any} */ (window).changed({ clickKeysOn: { newValue: true } }, "local"));
    await click(page, ["Alt"]);
    expect(await sent(page)).toHaveLength(1);
    await page.evaluate(() => /** @type {any} */ (window).changed({ clickKeysOn: { newValue: false } }, "local"));
    await click(page, ["Alt"]);
    expect(await sent(page)).toHaveLength(1);
    // An update or reload of the extension cuts the script off: the click
    // stays the browser's and the page's.
    await page.evaluate(() => /** @type {any} */ (window).changed({ clickKeysOn: { newValue: true } }, "local"));
    await page.evaluate(() => {
      /** @type {any} */ (window).chrome.runtime.id = undefined;
    });
    await click(page, ["Alt"]);
    expect(await sent(page)).toHaveLength(1);
    expect((await pageSaw(page)).at(-1)).toBe(false);
    await context.close();
  });

  it("an SVG link counts like any other", async () => {
    const { page, context } = await pageWith({
      stored: { sendKeys: "alt" },
      body: `<svg width="300" height="100" style="position:absolute;left:50px;top:50px"><a id="a" href="/f.zip"><rect width="300" height="100"/></a></svg>`,
    });
    await click(page, ["Alt"]);
    expect(await sent(page)).toEqual([{ type: "click-link", href: "https://page.invalid/f.zip" }]);
    await context.close();
  });

  it("off by default (owner's decision): until turned on, every click is the browser's and the page's", async () => {
    const { page, context } = await pageWith({ stored: { useKeys: false } });
    await click(page, ["Alt"]);
    await click(page, ["Alt", "Shift"]);
    expect(await sent(page)).toEqual([]);
    expect(await pageSaw(page)).toEqual([false, false]);
    // Turned on in the settings: at once.
    await page.evaluate(() => /** @type {any} */ (window).changed({ settings: { newValue: { useKeys: true, sendKeys: "alt" } } }, "local"));
    await click(page, ["Alt"]);
    expect(await sent(page)).toEqual([{ type: "click-link", href: "https://files.invalid/a.zip" }]);
    await context.close();
  });
});
