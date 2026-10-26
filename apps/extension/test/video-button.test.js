// The "Download video" button (`video-button.js`) in a real browser: it
// shows over a video, also under a player's own controls, and a real click
// sends the background only the video's address. On CI, the runner's
// Chrome; here, Playwright's Chromium. No site is reached: every request
// the page makes is refused.
import { readFileSync } from "node:fs";
import { afterAll, beforeAll, describe, expect, it } from "vitest";
import { BROWSER_START, launch } from "./browser.js";

const script = readFileSync(new URL("../src/video-button.js", import.meta.url), "utf8");

/** @type {import("playwright-core").Browser} */
let browser;

// Starting the runner's Chrome took from 0.2 s to 13.7 s on CI (#174):
// 30 s, not Vitest's 10.
beforeAll(async () => {
  browser = await launch("video-button");
}, BROWSER_START);

afterAll(async () => {
  await browser?.close();
});

/** A page with `body`, and the button's script with a fake `chrome` that
 * records what is sent. `open`: the test opens the button's shadow root, to
 * reach the button as no page can.
 * @param {string} body @param {{ open?: boolean }} [how] */
async function pageWith(body, how = {}) {
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  // The page itself is served here; anything else it asks for is refused.
  await page.route("**/*", (route) =>
    route.request().url() === "https://page.invalid/"
      ? route.fulfill({ contentType: "text/html", body: `<!doctype html><body style="margin:0">${body}</body>` })
      : route.abort(),
  );
  await page.addInitScript(() => {
    /** @type {any} */ (window).sent = [];
    /** @type {any} */ (window).chrome = {
      runtime: {
        id: "zajil",
        sendMessage: async (/** @type {unknown} */ m) => /** @type {any} */ (window).sent.push(m),
      },
      i18n: { getMessage: (/** @type {string} */ k) => ({ videoButton: "نزّل الفيديو", lang: "ar" })[k] ?? "" },
    };
  });
  if (how.open) {
    await page.addInitScript(() => {
      const attach = Element.prototype.attachShadow;
      Element.prototype.attachShadow = function () {
        return attach.call(this, { mode: "open" });
      };
    });
  }
  await page.goto("https://page.invalid/");
  await page.addScriptTag({ content: script });
  return page;
}

const sent = (/** @type {import("playwright-core").Page} */ page) => page.evaluate(() => /** @type {any} */ (window).sent);

const VIDEO = `<video src="https://media.invalid/v.mp4" style="position:absolute;left:100px;top:50px;width:640px;height:360px"></video>`;

describe("the Download video button", () => {
  it("shows over a video, and a click sends only its address", async () => {
    const page = await pageWith(VIDEO);
    await page.mouse.move(400, 200);
    // The button is at the video's corner (left 100 + 8, top 50 + 8).
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([{ type: "video-button", src: "https://media.invalid/v.mp4" }]);
    await page.close();
  });

  it("shows under a player's own controls covering the video", async () => {
    const page = await pageWith(`${VIDEO}<div style="position:absolute;left:100px;top:50px;width:640px;height:360px;z-index:5"></div>`);
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  // ADR-0009, section 6: a video played through EME (DRM) gets no button.
  it("does not show over a video that fired encrypted (EME)", async () => {
    const page = await pageWith(VIDEO);
    await page.evaluate(() => document.querySelector("video")?.dispatchEvent(new Event("encrypted")));
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("does not show over a video with media keys (EME), even when they come after the button showed", async () => {
    const page = await pageWith(VIDEO);
    // ClearKey: the EME system every browser has, no real DRM needed.
    const setKeys = () =>
      page.evaluate(async () => {
        const access = await navigator.requestMediaKeySystemAccess("org.w3.clearkey", [
          { initDataTypes: ["keyids"], videoCapabilities: [{ contentType: 'video/webm; codecs="vp8"' }] },
        ]);
        await document.querySelector("video")?.setMediaKeys(await access.createMediaKeys());
        return document.querySelector("video")?.mediaKeys != null;
      });
    // Shown first; the keys come before the click: nothing is sent.
    await page.mouse.move(400, 200);
    expect(await setKeys()).toBe(true);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([]);
    // And no button over it from now on.
    await page.mouse.move(10, 600);
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("does not show over a small video (an icon, a preview)", async () => {
    const page = await pageWith(`<video src="https://media.invalid/v.mp4" style="position:absolute;left:100px;top:50px;width:150px;height:100px"></video>`);
    await page.mouse.move(150, 90);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("goes away once the pointer has left the video", async () => {
    const page = await pageWith(VIDEO);
    // What is at the button's place, without moving the pointer there (that
    // would bring it back): the button's host, or the video.
    const atCorner = () => page.evaluate(() => document.elementFromPoint(120, 66)?.tagName);
    await page.mouse.move(400, 200);
    expect(await atCorner()).toBe("DIV");
    await page.mouse.move(900, 600);
    expect(await atCorner()).toBe("DIV");
    await page.waitForFunction(() => document.elementFromPoint(120, 66)?.tagName === "VIDEO", null, { timeout: 5000 });
    await page.close();
  });

  it("is out of the page's reach: no button in its document, and a closed root", async () => {
    const page = await pageWith(VIDEO);
    await page.mouse.move(400, 200);
    const seen = await page.evaluate(() => {
      const host = document.elementFromPoint(120, 66);
      /** @type {any} */ (host)?.click();
      host?.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
      return {
        buttons: document.querySelectorAll("button").length,
        open: host?.shadowRoot !== null,
        isVideo: host instanceof HTMLVideoElement,
      };
    });
    expect(seen).toEqual({ buttons: 0, open: false, isVideo: false });
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("a click a script makes on the button itself sends nothing: only a real click counts", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.mouse.move(400, 200);
    await page.waitForFunction(() => document.elementFromPoint(120, 66)?.tagName === "DIV");
    const reached = await page.evaluate(() => {
      const button = document.elementFromPoint(120, 66)?.shadowRoot?.querySelector("button");
      button?.click();
      button?.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
      return Boolean(button);
    });
    expect(reached).toBe(true);
    expect(await sent(page)).toEqual([]);
    // And the same button, clicked for real, sends.
    await page.mouse.click(120, 66);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  it("the page's own CSS cannot hide or move it", async () => {
    const style = `<style>div { display: none !important; opacity: 0 !important; visibility: hidden !important; }</style>`;
    const page = await pageWith(`${style}${VIDEO}`);
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  it("a page made unseen as a whole gets no press", async () => {
    const page = await pageWith(`<style>html { opacity: 0; }</style>${VIDEO}`);
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("an address longer than Zajil takes is left out, not cut", async () => {
    const long = `https://media.invalid/${"a".repeat(9000)}.mp4`;
    const page = await pageWith(`<video src="${long}" style="position:absolute;left:100px;top:50px;width:640px;height:360px"></video>`);
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toEqual([{ type: "video-button", src: "" }]);
    await page.close();
  });

  it("runs once in a page, even if injected twice", async () => {
    const page = await pageWith(VIDEO);
    await page.addScriptTag({ content: script });
    await page.mouse.move(400, 200);
    await page.mouse.click(120, 66);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });
});
