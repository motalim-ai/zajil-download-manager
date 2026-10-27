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
 * reach the button as no page can. `url`: where the page is. The
 * background's answers: `window.texts` to the texts asked for (Zajil's, in
 * Arabic, by default; a function's promise when it is one), `window.press()`
 * to a press (taken by default).
 * @param {string} body @param {{ open?: boolean, url?: string }} [how] */
async function pageWith(body, how = {}) {
  const url = how.url ?? "https://page.invalid/";
  const page = await browser.newPage({ viewport: { width: 1000, height: 700 } });
  // The page itself is served here; anything else it asks for is refused.
  await page.route("**/*", (route) =>
    route.request().url() === url
      ? route.fulfill({ contentType: "text/html", body: `<!doctype html><body style="margin:0">${body}</body>` })
      : route.abort(),
  );
  await page.addInitScript(() => {
    const w = /** @type {any} */ (window);
    w.sent = [];
    w.texts = { lang: "ar", texts: { button: "نزّل الفيديو", working: "يُرسَل إلى زاجِل…", done: "أُرسل إلى زاجِل", failed: "✗" } };
    w.press = async () => ({ taken: true });
    w.chrome = {
      runtime: {
        id: "zajil",
        sendMessage: async (/** @type {any} */ m) => {
          w.sent.push(m);
          if (m.type === "video-button-texts") return typeof w.texts === "function" ? w.texts() : w.texts;
          if (m.type === "video-button") return w.press();
        },
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
  await page.goto(url);
  await page.addScriptTag({ content: script });
  return page;
}

/** The presses sent (not the asks for texts). */
const sent = (/** @type {import("playwright-core").Page} */ page) =>
  page.evaluate(() => /** @type {any} */ (window).sent.filter((/** @type {any} */ m) => m.type === "video-button"));

/** Inside the button, at the video's top right: the video's right edge is
 * at 740, the button's at 740 - 8. */
const AT = { x: 715, y: 66 };

/** The button as no page can reach it (`open` pages only). */
const buttonOf = (/** @type {import("playwright-core").Page} */ page) =>
  page.evaluate(() => {
    const button = [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button");
    if (!button) return null;
    const box = button.getBoundingClientRect();
    return {
      hidden: button.hidden,
      state: button.dataset.state,
      busy: button.getAttribute("aria-busy"),
      dir: button.dir,
      text: button.textContent,
      right: Math.round(box.right),
      top: Math.round(box.top),
      mark: button.querySelectorAll("svg path").length,
    };
  });

const VIDEO = `<video src="https://media.invalid/v.mp4" style="position:absolute;left:100px;top:50px;width:640px;height:360px"></video>`;

describe("the Download video button", () => {
  it("shows over a video, and a click sends only its address", async () => {
    const page = await pageWith(VIDEO);
    await page.mouse.move(400, 200);
    // The button is at the video's top right (740 - 8, top 50 + 8).
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([{ type: "video-button", src: "https://media.invalid/v.mp4" }]);
    await page.close();
  });

  it("shows under a player's own controls covering the video", async () => {
    const page = await pageWith(`${VIDEO}<div style="position:absolute;left:100px;top:50px;width:640px;height:360px;z-index:5"></div>`);
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  // ADR-0009, section 6: a video played through EME (DRM) gets no button.
  it("does not show over a video that fired encrypted (EME)", async () => {
    const page = await pageWith(VIDEO);
    await page.evaluate(() => document.querySelector("video")?.dispatchEvent(new Event("encrypted")));
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
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
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([]);
    // And no button over it from now on.
    await page.mouse.move(10, 600);
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("does not show over a small video (an icon, a preview)", async () => {
    const page = await pageWith(`<video src="https://media.invalid/v.mp4" style="position:absolute;left:100px;top:50px;width:150px;height:100px"></video>`);
    await page.mouse.move(150, 90);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("goes away once the pointer has left the video", async () => {
    const page = await pageWith(VIDEO);
    // What is at the button's place, without moving the pointer there (that
    // would bring it back): the button's host, or the video.
    const atCorner = () => page.evaluate(() => document.elementFromPoint(715, 66)?.tagName);
    await page.mouse.move(400, 200);
    expect(await atCorner()).toBe("DIV");
    await page.mouse.move(900, 600);
    expect(await atCorner()).toBe("DIV");
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "VIDEO", null, { timeout: 5000 });
    await page.close();
  });

  it("is out of the page's reach: no button in its document, and a closed root", async () => {
    const page = await pageWith(VIDEO);
    await page.mouse.move(400, 200);
    const seen = await page.evaluate(() => {
      const host = document.elementFromPoint(715, 66);
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
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "DIV");
    const reached = await page.evaluate(() => {
      const button = document.elementFromPoint(715, 66)?.shadowRoot?.querySelector("button");
      button?.click();
      button?.dispatchEvent(new MouseEvent("click", { bubbles: true, composed: true }));
      return Boolean(button);
    });
    expect(reached).toBe(true);
    expect(await sent(page)).toEqual([]);
    // And the same button, clicked for real, sends.
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  it("the page's own CSS cannot hide or move it", async () => {
    const style = `<style>div { display: none !important; opacity: 0 !important; visibility: hidden !important; }</style>`;
    const page = await pageWith(`${style}${VIDEO}`);
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  it("a page made unseen as a whole gets no press", async () => {
    const page = await pageWith(`<style>html { opacity: 0; }</style>${VIDEO}`);
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([]);
    await page.close();
  });

  it("an address longer than Zajil takes is left out, not cut", async () => {
    const long = `https://media.invalid/${"a".repeat(9000)}.mp4`;
    const page = await pageWith(`<video src="${long}" style="position:absolute;left:100px;top:50px;width:640px;height:360px"></video>`);
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([{ type: "video-button", src: "" }]);
    await page.close();
  });

  it("runs once in a page, even if injected twice", async () => {
    const page = await pageWith(VIDEO);
    await page.addScriptTag({ content: script });
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toHaveLength(1);
    await page.close();
  });

  it("sits at the video's top right, with Zajil's mark, in either language", async () => {
    for (const lang of ["ar", "en"]) {
      const page = await pageWith(VIDEO, { open: true });
      if (lang === "en") await page.evaluate(() => void (/** @type {any} */ (window).texts = { lang: "en", texts: { button: "Download video", working: "Sending to Zajil…", done: "Sent to Zajil", failed: "The video did not reach Zajil." } }));
      await page.mouse.move(400, 200);
      const word = lang === "en" ? "Download video" : "نزّل الفيديو";
      await page.waitForFunction((w) => [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button")?.textContent === w, word);
      const button = await buttonOf(page);
      expect(button).toMatchObject({ hidden: false, right: 732, top: 58, mark: 3, dir: lang === "en" ? "ltr" : "rtl" });
      await page.close();
    }
  });

  it("speaks Zajil's language, not the browser's", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.evaluate(() => void (/** @type {any} */ (window).texts = { lang: "en", texts: { button: "Download video", working: "Sending to Zajil…", done: "Sent to Zajil", failed: "The video did not reach Zajil." } }));
    await page.mouse.move(400, 200);
    await page.waitForFunction(() => {
      const button = [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button");
      return button?.textContent === "Download video";
    });
    expect(await buttonOf(page)).toMatchObject({ dir: "ltr", text: "Download video" });
    await page.close();
  });

  it("shows nothing until Zajil's texts come, then only in Zajil's language", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.evaluate(() => {
      const w = /** @type {any} */ (window);
      w.texts = () => new Promise((give) => (w.give = give));
    });
    await page.mouse.move(400, 200);
    await page.waitForFunction(() => typeof (/** @type {any} */ (window).give) === "function");
    // Asked, and not shown in the meantime, in no language.
    expect((await buttonOf(page))?.hidden ?? true).toBe(true);
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toEqual([]);
    await page.evaluate(() =>
      /** @type {any} */ (window).give({ lang: "en", texts: { button: "Download video", working: "Sending to Zajil…", done: "Sent to Zajil", failed: "The video did not reach Zajil." } }),
    );
    // Then shown over the video the pointer is on, in Zajil's language.
    await page.waitForFunction(() => {
      const button = [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button");
      return button && !button.hidden && button.textContent === "Download video";
    });
    expect(await buttonOf(page)).toMatchObject({ dir: "ltr", right: 732 });
    await page.close();
  });

  it("never shows when Zajil's texts do not come", async () => {
    for (const texts of [null, "reject"]) {
      const page = await pageWith(VIDEO, { open: true });
      await page.evaluate((how) => {
        const w = /** @type {any} */ (window);
        w.texts = how === "reject" ? () => Promise.reject(new Error("gone")) : null;
      }, texts);
      await page.mouse.move(400, 200);
      await page.waitForTimeout(200);
      await page.mouse.click(AT.x, AT.y);
      expect((await buttonOf(page))?.hidden ?? true, String(texts)).toBe(true);
      expect(await sent(page)).toEqual([]);
      await page.close();
    }
  });

  it("pressed, it stays and says it is working until Zajil answers", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.evaluate(() => {
      const w = /** @type {any} */ (window);
      w.press = () => new Promise((done) => (w.answer = done));
    });
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    // The pointer leaves, and the page scrolls: it stays.
    await page.mouse.move(900, 650);
    await page.waitForTimeout(1800);
    expect(await buttonOf(page)).toMatchObject({ hidden: false, state: "working", busy: "true" });
    // A second press while it works sends nothing more.
    await page.mouse.click(AT.x, AT.y);
    expect(await sent(page)).toHaveLength(1);
    await page.evaluate(() => /** @type {any} */ (window).answer({ taken: true }));
    await page.waitForFunction(() => [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button")?.dataset.state === "done");
    await page.close();
  });

  it("says why when Zajil does not take the video, or cannot be reached", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.evaluate(() => void (/** @type {any} */ (window).press = async () => ({ taken: false, error: "زاجِل لا يحمّل هذا الفيديو." })));
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    await page.waitForFunction(() => [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button")?.dataset.state === "error");
    expect(await buttonOf(page)).toMatchObject({ hidden: false, text: "زاجِل لا يحمّل هذا الفيديو." });
    await page.close();
    // The background gone (the extension updated under the page): its own text.
    const gone = await pageWith(VIDEO, { open: true });
    await gone.evaluate(() => void (/** @type {any} */ (window).press = async () => Promise.reject(new Error("Extension context invalidated."))));
    await gone.mouse.move(400, 200);
    await gone.mouse.click(AT.x, AT.y);
    await gone.waitForFunction(() => [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button")?.dataset.state === "error");
    expect(await buttonOf(gone)).toMatchObject({ text: "✗" });
    await gone.close();
  });

  // Updated, reloaded or turned off, the extension leaves the script in open
  // tabs, cut off: `runtime.id` gone, and `sendMessage` throwing at once.
  /** Cuts the script off as the browser does. @param {import("playwright-core").Page} page */
  const cutOff = (page) =>
    page.evaluate(() => {
      const runtime = /** @type {any} */ (window).chrome.runtime;
      runtime.id = undefined;
      runtime.sendMessage = () => {
        throw new Error("Extension context invalidated.");
      };
    });
  /** The page's uncaught errors, from now on. @param {import("playwright-core").Page} page */
  const errorsOf = (page) => {
    /** @type {string[]} */
    const errors = [];
    page.on("pageerror", (error) => errors.push(error.message));
    return errors;
  };

  it("once the extension is gone, the button and its listeners go, with no error in the page", async () => {
    const page = await pageWith(VIDEO);
    const errors = errorsOf(page);
    await page.mouse.move(400, 200);
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "DIV");
    await cutOff(page);
    await page.mouse.move(410, 210);
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "VIDEO", null, { timeout: 2000 });
    // Gone for good: the extension back, the script still does nothing.
    await page.evaluate(() => void (/** @type {any} */ (window).chrome.runtime.id = "zajil"));
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    await page.waitForTimeout(100);
    expect(await page.evaluate(() => document.elementFromPoint(715, 66)?.tagName)).toBe("VIDEO");
    expect(await sent(page)).toEqual([]);
    expect(errors).toEqual([]);
    await page.close();
  });

  it("gone before the first video: asking for the texts throws nothing into the page", async () => {
    const page = await pageWith(VIDEO, { open: true });
    const errors = errorsOf(page);
    await cutOff(page);
    await page.mouse.move(400, 200);
    await page.waitForTimeout(100);
    expect((await buttonOf(page))?.hidden ?? true).toBe(true);
    expect(errors).toEqual([]);
    await page.close();
    // `sendMessage` throwing at once while `runtime.id` is still there: no
    // error either, and no button without the texts.
    const still = await pageWith(VIDEO, { open: true });
    const stillErrors = errorsOf(still);
    await still.evaluate(() => {
      /** @type {any} */ (window).chrome.runtime.sendMessage = () => {
        throw new Error("Extension context invalidated.");
      };
    });
    await still.mouse.move(400, 200);
    await still.waitForTimeout(100);
    expect((await buttonOf(still))?.hidden ?? true).toBe(true);
    expect(stillErrors).toEqual([]);
    await still.close();
  });

  it("gone between showing and the press: the press sends nothing and the button goes", async () => {
    const page = await pageWith(VIDEO);
    const errors = errorsOf(page);
    await page.mouse.move(400, 200);
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "DIV");
    await cutOff(page);
    await page.mouse.click(AT.x, AT.y);
    await page.waitForFunction(() => document.elementFromPoint(715, 66)?.tagName === "VIDEO", null, { timeout: 2000 });
    expect(await sent(page)).toEqual([]);
    expect(errors).toEqual([]);
    await page.close();
  });

  it("a pointer move the page makes shows nothing and asks nothing", async () => {
    const page = await pageWith(VIDEO, { open: true });
    await page.evaluate(() => {
      document.dispatchEvent(new PointerEvent("pointermove", { clientX: 400, clientY: 200, bubbles: true }));
    });
    await page.waitForTimeout(100);
    // Never shown: not even put in the page.
    expect((await buttonOf(page))?.hidden ?? true).toBe(true);
    expect(await page.evaluate(() => /** @type {any} */ (window).sent)).toEqual([]);
    await page.close();
  });

  it("pressed, it follows its video when the page scrolls", async () => {
    const page = await pageWith(`<div style="height:2000px">${VIDEO}</div>`, { open: true });
    await page.evaluate(() => {
      const w = /** @type {any} */ (window);
      w.press = () => new Promise(() => {});
    });
    await page.mouse.move(400, 200);
    await page.mouse.click(AT.x, AT.y);
    await page.evaluate(() => window.scrollBy(0, 30));
    await page.waitForFunction(() => {
      const button = [...document.documentElement.children].at(-1)?.shadowRoot?.querySelector("button");
      return button && Math.round(button.getBoundingClientRect().top) === 28;
    });
    expect(await buttonOf(page)).toMatchObject({ hidden: false, state: "working" });
    await page.close();
  });

  it("on YouTube, only on a video's page, a Short or an embedded player", async () => {
    for (const [url, shown] of /** @type {[string, boolean][]} */ ([
      ["https://www.youtube.com/", false],
      ["https://www.youtube.com/results?search_query=zajil", false],
      ["https://www.youtube.com/@channel/videos", false],
      ["https://www.youtube.com/watch?v=abc", true],
      ["https://m.youtube.com/shorts/abc", true],
      ["https://www.youtube-nocookie.com/embed/abc", true],
      ["https://www.youtube.com/live/abc", true],
      // Not YouTube: shown as on any site.
      ["https://youtube.com.example.test/", true],
    ])) {
      const page = await pageWith(VIDEO, { url });
      await page.mouse.move(400, 200);
      await page.mouse.click(AT.x, AT.y);
      expect(await sent(page), url).toHaveLength(shown ? 1 : 0);
      await page.close();
    }
  });
});

describe("Zajil's mark in the button", () => {
  it("is the mark of assets/icon/zajil-mark.svg, drawn by the script", () => {
    const svg = readFileSync(new URL("../../../assets/icon/zajil-mark.svg", import.meta.url), "utf8");
    const paths = [...svg.matchAll(/<path d="([^"]+)"/g)].map((m) => m[1]);
    const viewBox = svg.match(/viewBox="([^"]+)"/)?.[1];
    expect(paths).toHaveLength(3);
    for (const d of paths) expect(script).toContain(`"${d}"`);
    expect(script).toContain(`const MARK_VIEWBOX = "${viewBox}";`);
    // Drawn in the page, never loaded from the extension's files.
    expect(script).toContain("createElementNS");
  });
});
