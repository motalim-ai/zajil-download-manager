import { readFileSync } from "node:fs";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";

/** A fake `chrome` for the real `background.js`: its listeners are kept to
 * be fired, and what it does is recorded. `granted`: the cookie
 * permission; `buttons`: whether notifications take buttons (Firefox:
 * not); `answer`: what Zajil answers. */
function fakeChrome({ granted, video = false, buttons = true, answer, jar = () => [{ name: "sid", value: "abc" }], windows = [], tabs = {} }) {
  const listeners = {};
  const on = (name) => ({ addListener: (fn) => (listeners[name] = fn) });
  const session = {};
  const done = { tabs: [], notifications: [], downloads: [], sent: [], cleared: [], cookieReads: [], ports: [], downloadOptions: [], menus: [], scripts: [], watched: [], tabOptions: [], order: [] };
  // What "Allow Zajil on sites" gave: the cookies, the video features.
  const have = new Set([...(granted ? ["cookies"] : []), ...(video ? ["scripting", "webRequest"] : [])]);
  const local = {};
  const chrome = {
    i18n: { getMessage: (key) => key },
    runtime: {
      id: "zajil",
      getURL: (path) => `chrome-extension://zajil/${path}`,
      onInstalled: on("installed"),
      onStartup: on("startup"),
      // The link to Zajil (M5): its port, recorded.
      connectNative: vi.fn(() => {
        const port = {
          sent: /** @type {any[]} */ ([]),
          onMessage: on("portMessage"),
          onDisconnect: on("portDisconnect"),
          postMessage: (m) => port.sent.push(m),
        };
        done.ports.push(port);
        return port;
      }),
      openOptionsPage: async () => {},
      onMessage: on("message"),
      sendNativeMessage: vi.fn(async (_host, message) => {
        done.sent.push(message);
        return typeof answer === "function" ? answer(message) : answer;
      }),
    },
    permissions: {
      contains: async (p) => p.permissions.every((x) => have.has(x)),
      onAdded: on("permissionsAdded"),
      onRemoved: on("permissionsRemoved"),
    },
    // Only there with the permission, as in Chrome.
    webRequest: video
      ? {
          onHeadersReceived: {
            addListener: (fn, filter, extra) => {
              listeners.headers = fn;
              done.watched.push({ filter, extra });
            },
            hasListener: (fn) => listeners.headers === fn,
            removeListener: () => delete listeners.headers,
          },
        }
      : undefined,
    scripting: video
      ? {
          getRegisteredContentScripts: async ({ ids }) => done.scripts.filter((s) => ids.includes(s.id)),
          registerContentScripts: async (list) => done.scripts.push(...list),
          unregisterContentScripts: async ({ ids }) => done.scripts.splice(0, done.scripts.length, ...done.scripts.filter((s) => !ids.includes(s.id))),
        }
      : undefined,
    cookies: {
      getAll: async (query) => {
        done.cookieReads.push(query);
        return jar(query);
      },
    },
    storage: {
      local: {
        get: async (key) => (typeof key === "string" ? { [key]: local[key] } : { ...local }),
        set: async (value) => Object.assign(local, value),
      },
      session: {
        get: async (key) => (key === null ? { ...session } : { [key]: session[key] }),
        set: async (value) => Object.assign(session, value),
        remove: async (key) => [key].flat().forEach((k) => delete session[k]),
      },
    },
    tabs: {
      create: async (tab) => {
        done.tabs.push(tab.url);
        done.tabOptions.push(tab);
      },
      get: async (id) => tabs[id] ?? { id, incognito: false },
      onRemoved: on("tabRemoved"),
    },
    windows: { onRemoved: on("windowRemoved"), getAll: async () => windows },
    action: { setBadgeText: async () => {}, onClicked: on("action") },
    contextMenus: { create: (item) => done.menus.push(item), onClicked: on("menu") },
    downloads: {
      onDeterminingFilename: on("filename"),
      setUiOptions: async (options) => void done.order.push(options.enabled ? "ui shown" : "ui hidden"),
      download: async (options) => {
        done.downloads.push(options.url);
        done.downloadOptions.push(options);
      },
      pause: async () => {},
      resume: async () => {},
      cancel: async () => {},
      erase: async () => {},
    },
    notifications: {
      create: async (options) => {
        if (options.buttons && !buttons) throw new Error("buttons is not supported");
        done.notifications.push(options);
        return `n${done.notifications.length}`;
      },
      clear: async (id) => done.cleared.push(id),
      onClicked: on("clicked"),
      onClosed: on("closed"),
      onButtonClicked: buttons ? on("button") : undefined,
    },
  };
  return { chrome, listeners, done, session, local, have };
}

const flush = () => new Promise((resolve) => setTimeout(resolve, 20));

async function load(fake) {
  vi.resetModules();
  globalThis.chrome = fake.chrome;
  await import("../src/background.js");
}

afterEach(() => {
  delete globalThis.chrome;
});

const forbidden = { protocol_version: 1, ok: false, error: "Zajil cannot download it: the server answered 403 Forbidden" };

describe("the background", () => {
  it("starts with the browser, and opens its link to Zajil at once (review of #80)", async () => {
    const fake = fakeChrome({ granted: false, answer: { ok: true } });
    await load(fake);
    await flush();
    expect(fake.listeners.startup).toBeTypeOf("function");
    expect(fake.done.ports).toHaveLength(1);
    expect(fake.done.ports[0].sent[0]).toMatchObject({ type: "link", protocol_version: 1 });
  });

  it("hides the browser's download UI before Chrome settles a download Zajil takes, and not otherwise", async () => {
    const fake = fakeChrome({ granted: false, answer: { ok: true } });
    await load(fake);
    await flush();
    // Shown again at every start, in case a stopped background left it hidden.
    expect(fake.done.order).toEqual(["ui shown"]);
    fake.done.order.length = 0;
    // Taken: hidden, then settled, then shown once Zajil has it.
    const suggested = vi.fn(() => fake.done.order.push("suggest"));
    const back = fake.listeners.filename({ id: 8, url: "https://example.com/a.zip", finalUrl: "https://example.com/a.zip", state: "in_progress" }, suggested);
    expect(back).toBe(true);
    expect(suggested).not.toHaveBeenCalled();
    await flush();
    expect(fake.done.order).toEqual(["ui hidden", "suggest", "ui shown"]);
    expect(fake.done.sent.at(-1)).toMatchObject({ type: "download", url: "https://example.com/a.zip" });
    // Not a listed type: settled as it is, the UI never touched.
    fake.done.order.length = 0;
    const page = vi.fn(() => fake.done.order.push("suggest"));
    fake.listeners.filename({ id: 9, url: "https://example.com/page.html", finalUrl: "https://example.com/page.html", state: "in_progress" }, page);
    await flush();
    expect(fake.done.order).toEqual(["suggest"]);
    expect(page).toHaveBeenCalledTimes(1);
  });

  it("keeps a ticket only for a download Zajil took with cookies, with its store", async () => {
    const tickets = (fake) => Object.keys(fake.local.linkTickets ?? {});
    // Taken with cookies, from a container: its ticket, as it was sent.
    let fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.filename(
      { id: 5, url: "https://example.com/a.zip", finalUrl: "https://example.com/a.zip", state: "in_progress", cookieStoreId: "firefox-container-1" },
      () => {},
    );
    await flush();
    expect(tickets(fake)).toEqual([fake.done.sent[0].ticket]);
    const { openTicket } = await import("../src/link.js");
    expect(await openTicket(fake.chrome, fake.done.sent[0].ticket, "https://example.com/b.zip", "firefox-container-1")).toBe(true);
    expect(await openTicket(fake.chrome, fake.done.sent[0].ticket, "https://example.com/b.zip", undefined)).toBe(false);
    // From the menu, in a container: the same.
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/c.zip" }, { cookieStoreId: "firefox-container-2" });
    await flush();
    expect(tickets(fake)).toContain(fake.done.sent[1].ticket);
    expect(await openTicket(fake.chrome, fake.done.sent[1].ticket, "https://example.com/c.zip", "firefox-container-2")).toBe(true);
    // Refused by Zajil, or sent without cookies: no ticket kept.
    fake = fakeChrome({ granted: true, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(tickets(fake)).toEqual([]);
    fake = fakeChrome({ granted: false, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(fake.done.sent[0].ticket).toMatch(/^[0-9a-f-]{36}$/);
    expect(tickets(fake)).toEqual([]);
  });

  it("keeps the ticket before Zajil is asked, and while no answer came back (#158)", async () => {
    // Zajil asks for the file's download address's cookies while it takes
    // the download (a Google Drive link): the ticket must be there by then.
    const kept = [];
    const fake = fakeChrome({ granted: true, answer: { ok: false, error: "Zajil did not answer in time" } });
    await load(fake);
    fake.chrome.runtime.sendNativeMessage.mockImplementation(async (_host, message) => {
      fake.done.sent.push(message);
      kept.push(Object.keys(fake.local.linkTickets ?? {}).includes(message.ticket));
      return { ok: false, error: "Zajil did not answer in time" };
    });
    fake.listeners.filename({ id: 7, url: "https://example.com/e.zip", finalUrl: "https://example.com/e.zip", state: "in_progress" }, () => {});
    await flush();
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/d.zip" });
    await flush();
    expect(kept).toEqual([true, true]);
    // No answer: Zajil may have taken them, so both tickets stay.
    expect(Object.keys(fake.local.linkTickets ?? {}).sort()).toEqual(fake.done.sent.map((m) => m.ticket).sort());
  });

  it("opens the welcome page once, at install", async () => {
    const fake = fakeChrome({ granted: false, answer: { ok: true } });
    await load(fake);
    fake.listeners.installed({ reason: "install" });
    fake.listeners.installed({ reason: "update" });
    await flush();
    // An unpacked extension is "installed" again at each start: not again.
    fake.listeners.installed({ reason: "install" });
    await flush();
    expect(fake.done.tabs).toEqual(["chrome-extension://zajil/welcome.html"]);
  });

  it("without the permission: sends no cookies; a sign-in site offers to allow them and to download with the browser", async () => {
    const fake = fakeChrome({ granted: false, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip", pageUrl: "https://example.com/" });
    await flush();
    expect(fake.done.sent[0].cookies).toBeUndefined();
    const notice = fake.done.notifications.at(-1);
    expect(notice.message).toBe("menuSignInNeedsCookies");
    expect(notice.buttons.map((b) => b.title)).toEqual(["cookiesAllow", "downloadWithBrowser"]);
    fake.listeners.button("n1", 1);
    await flush();
    expect(fake.done.downloads).toEqual(["https://example.com/a.zip"]);
    fake.listeners.button("n1", 0);
    await flush();
    expect(fake.done.tabs).toEqual([], "the notice was used once");
  });

  it("with the permission: sends the link's cookies; a site that still refuses says so, with no cookie button", async () => {
    const fake = fakeChrome({ granted: true, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(fake.done.sent[0].cookies).toBe("sid=abc");
    const notice = fake.done.notifications.at(-1);
    expect(notice.message).toBe("menuSignInEvenWithCookies");
    expect(notice.buttons.map((b) => b.title)).toEqual(["downloadWithBrowser"]);
  });

  it("a captured download from a sign-in site, without the permission, offers only to allow them", async () => {
    const fake = fakeChrome({ granted: false, answer: forbidden });
    await load(fake);
    fake.listeners.filename({ id: 3, url: "https://example.com/a.zip", finalUrl: "https://example.com/a.zip", state: "in_progress" }, () => {});
    await flush();
    const notice = fake.done.notifications.at(-1);
    expect(notice.message).toBe("signInNeedsCookies");
    expect(notice.buttons.map((b) => b.title)).toEqual(["cookiesAllow"]);
    fake.listeners.button("n1", 0);
    await flush();
    expect(fake.done.tabs).toEqual(["chrome-extension://zajil/welcome.html"]);
  });

  it("reads the cookies of the download's own identity (a Firefox container), never the default", async () => {
    const jar = ({ storeId }) => [{ name: "who", value: storeId ?? "default" }];
    const fake = fakeChrome({ granted: true, answer: { ok: true }, jar });
    await load(fake);
    fake.listeners.filename(
      { id: 4, url: "https://example.com/a.zip", finalUrl: "https://example.com/a.zip", state: "in_progress", cookieStoreId: "firefox-container-1" },
      () => {},
    );
    await flush();
    expect(fake.done.sent[0].cookies).toBe("who=firefox-container-1");
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/b.zip" }, { cookieStoreId: "firefox-container-2" });
    await flush();
    expect(fake.done.sent[1].cookies).toBe("who=firefox-container-2");
    // Where they came from goes too, for after a restart (M5).
    expect(fake.done.sent[0].store).toBe("firefox-container-1");
    expect(fake.done.sent[1].store).toBe("firefox-container-2");
    expect(fake.done.sent[0].instance).toMatch(/^[0-9a-f-]{36}$/);
    expect(fake.done.sent[1].instance).toBe(fake.done.sent[0].instance);
    // Only the download link's cookies are read, not the page's.
    expect(fake.done.cookieReads.map((q) => q.url)).toEqual(["https://example.com/a.zip", "https://example.com/b.zip"]);
  });

  it("from a private window, a sign-in site offers no button, and nothing is kept for one (review #84)", async () => {
    const fake = fakeChrome({ granted: false, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" }, { incognito: true });
    await flush();
    expect(fake.done.sent[0].private).toBe(true);
    expect(fake.done.sent[0].instance).toBeUndefined();
    expect(fake.done.notifications.at(-1).buttons).toBeUndefined();
    expect(fake.session).toEqual({});
    fake.listeners.clicked("n1");
    await flush();
    expect(fake.done.downloads).toEqual([]);
    expect(fake.done.tabs).toEqual([]);
  });

  it("tells Zajil when the last private window closes, once, and only after a private download (review of #91)", async () => {
    const windows = [{ incognito: true }, { incognito: false }];
    const fake = fakeChrome({ granted: false, answer: { ok: true }, windows });
    await load(fake);
    // No private download yet: nothing is sent when a window closes.
    fake.listeners.windowRemoved(1);
    await flush();
    expect(fake.done.sent).toEqual([]);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" }, { incognito: true });
    await flush();
    expect(fake.session).toEqual({ privateSent: true });
    // A private window is still open: not yet.
    fake.listeners.windowRemoved(2);
    await flush();
    expect(fake.done.sent).toHaveLength(1);
    windows.splice(0, 1);
    fake.listeners.windowRemoved(3);
    await flush();
    expect(fake.done.sent.at(-1)).toEqual({ type: "private_closed", protocol_version: 1 });
    expect(fake.session).toEqual({});
    fake.listeners.windowRemoved(4);
    await flush();
    expect(fake.done.sent).toHaveLength(2);
  });

  it("downloads with the browser in the tab's own container (review #84)", async () => {
    const fake = fakeChrome({ granted: true, answer: forbidden, jar: () => [] });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" }, { cookieStoreId: "firefox-container-3" });
    await flush();
    expect(fake.done.sent[0].private).toBeUndefined();
    fake.listeners.button("n1", 0);
    await flush();
    expect(fake.done.downloadOptions.at(-1)).toEqual({ url: "https://example.com/a.zip", cookieStoreId: "firefox-container-3" });
  });

  it("from a private window, the menu sends no cookies and reads none", async () => {
    const fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" }, { incognito: true });
    await flush();
    expect(fake.done.sent[0].cookies).toBeUndefined();
    expect(fake.done.cookieReads).toEqual([]);
  });

  it("offers 'Download video with Zajil' on a video or audio, and sends its page with the page's cookies (M6)", async () => {
    const fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.installed({ reason: "update" });
    expect(fake.done.menus).toContainEqual({ id: "zajil-download-video", title: "menuDownloadVideo", contexts: ["video", "audio"] });
    fake.listeners.menu(
      { menuItemId: "zajil-download-video", pageUrl: "https://example.com/watch?v=1", srcUrl: "blob:https://example.com/9" },
      { cookieStoreId: "firefox-container-1" },
    );
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://example.com/watch?v=1", cookies: "sid=abc" }]);
    expect(fake.done.cookieReads.at(-1)).toMatchObject({ url: "https://example.com/watch?v=1", storeId: "firefox-container-1" });
    // Nothing kept for a video: Zajil does not ask for its cookies again.
    expect(fake.local.linkTickets).toBeUndefined();
  });

  it("a video from a private window goes without cookies, none read, and marked private", async () => {
    const fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v", srcUrl: "https://cdn.example.com/v.mp4" }, { incognito: true });
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://example.com/v", media: "https://cdn.example.com/v.mp4", private: true }]);
    expect(fake.done.cookieReads).toEqual([]);
    expect(fake.session.privateSent).toBe(true);
  });

  it("a video Zajil does not take says Zajil's own reason, with no sign-in button: the browser cannot download a page's video", async () => {
    let fake = fakeChrome({ granted: false, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v" });
    await flush();
    expect(fake.done.notifications.at(-1)).toMatchObject({ message: forbidden.error });
    // «Coming soon», as the app says it (review of #99).
    fake = fakeChrome({ granted: false, answer: { protocol_version: 1, ok: false, error: "تحميل الفيديو قادم قريبًا." } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v" });
    await flush();
    expect(fake.done.notifications.at(-1)).toMatchObject({ message: "تحميل الفيديو قادم قريبًا." });
    expect(fake.done.notifications.at(-1).buttons).toBeUndefined();
    // A page Zajil cannot read: refused before any cookie is read.
    fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "file:///home/me/v.html", srcUrl: "file:///home/me/v.mp4" });
    await flush();
    expect(fake.done.sent).toEqual([]);
    expect(fake.done.cookieReads).toEqual([]);
    expect(fake.done.notifications.at(-1).message).toBe("videoNotWeb");
  });

  it("without 'Allow Zajil on sites': no button's script, no request watched; the menu only", async () => {
    const fake = fakeChrome({ granted: false, answer: { ok: true } });
    await load(fake);
    await flush();
    expect(fake.done.scripts).toEqual([]);
    expect(fake.listeners.headers).toBeUndefined();
  });

  it("with it: the button's script on every site and frame, and playlists watched, never blocked", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    expect(fake.done.scripts).toEqual([
      { id: "zajil-video-button", js: ["video-button.js"], matches: ["<all_urls>"], allFrames: true, runAt: "document_idle", persistAcrossSessions: true },
      // The keys of a click (owner's decision, 2026-10-26), from a page's start.
      { id: "zajil-click-keys", js: ["click-keys.js"], matches: ["<all_urls>"], allFrames: true, runAt: "document_start", persistAcrossSessions: true },
    ]);
    expect(fake.done.watched).toHaveLength(1);
    expect(fake.done.watched[0].extra).toEqual(["responseHeaders"]);
    // Taken back: the script goes, and nothing is watched any more.
    fake.have.delete("webRequest");
    fake.have.delete("scripting");
    fake.session["streams:3"] = [{ url: "https://a.example/master.m3u8?sig=1", kind: "hls", frame: 0 }];
    fake.session.privateSent = true;
    fake.listeners.permissionsRemoved({ permissions: ["scripting", "webRequest"] });
    await flush();
    expect(fake.done.scripts).toEqual([]);
    expect(fake.listeners.headers).toBeUndefined();
    // What was noted under the permission goes with it; nothing else.
    expect(fake.session).toEqual({ privateSent: true });
  });

  it("without the video permission, a playlist left from before is never sent (review)", async () => {
    // Taken back while its tab stayed open, so nothing forgot it.
    const fake = fakeChrome({ granted: true, video: false, answer: { ok: true } });
    await load(fake);
    await flush();
    fake.session["streams:5"] = [{ url: "https://a.example/master.m3u8?sig=1", kind: "hls", frame: 0 }];
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://b.example/v", srcUrl: "blob:https://b.example/1", frameId: 0 }, { id: 5 });
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://b.example/v", cookies: "sid=abc" }]);
  });

  it("the button sends the tab's page (the browser's word, not the page's) and, for a blob, the playlist the tab loaded", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    fake.listeners.headers({
      url: "https://cdn.example/master.m3u8?sig=1",
      tabId: 4,
      frameId: 0,
      type: "xmlhttprequest",
      initiator: "https://example.com",
      responseHeaders: [{ name: "Content-Type", value: "application/x-mpegURL" }],
    });
    await flush();
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/watch?v=1" }, frameId: 0, url: "https://example.com/watch?v=1" };
    fake.listeners.message({ type: "video-button", src: "blob:https://example.com/5" }, sender);
    await flush();
    expect(fake.done.sent).toEqual([
      { type: "video", protocol_version: 1, page: "https://example.com/watch?v=1", media: "https://cdn.example/master.m3u8?sig=1", cookies: "sid=abc" },
    ]);
    // The tab closed: its playlists are forgotten.
    fake.listeners.tabRemoved(4);
    await flush();
    expect(fake.session["streams:4"]).toBeUndefined();
  });

  it("the button in an embedded player sends the frame's page; its own address wins over a playlist", async () => {
    const fake = fakeChrome({ granted: false, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    const sender = { id: "zajil", tab: { id: 2, url: "https://blog.example/post" }, frameId: 3, url: "https://player.example/embed/9" };
    fake.listeners.message({ type: "video-button", src: "https://cdn.example/v.mp4" }, sender);
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://player.example/embed/9", media: "https://cdn.example/v.mp4" }]);
  });

  it("the button in a private window: no cookies, no playlist, marked private", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    fake.session["streams:6"] = [{ url: "https://cdn.example/a.m3u8", kind: "hls", frame: 0 }];
    const sender = { id: "zajil", tab: { id: 6, url: "https://example.com/v", incognito: true }, frameId: 0, url: "https://example.com/v" };
    fake.listeners.message({ type: "video-button", src: "blob:https://example.com/1" }, sender);
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://example.com/v", private: true }]);
    expect(fake.done.cookieReads).toEqual([]);
  });

  describe("a video read ahead, when the button shows (owner's decision, 2026-10-28)", () => {
    it("sends peek_video as a press would send video, once a page in ten minutes, answering nothing", async () => {
      vi.useFakeTimers({ toFake: ["Date"] });
      try {
        const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
        await load(fake);
        await flush();
        fake.listeners.headers({
          url: "https://cdn.example/master.m3u8?sig=1",
          tabId: 4,
          frameId: 0,
          type: "xmlhttprequest",
          initiator: "https://example.com",
          responseHeaders: [{ name: "Content-Type", value: "application/x-mpegURL" }],
        });
        await flush();
        const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/watch?v=1" }, frameId: 0, url: "https://example.com/watch?v=1" };
        expect(fake.listeners.message({ type: "video-peek", src: "blob:https://example.com/5" }, sender)).toBeUndefined();
        await flush();
        const peek = { type: "peek_video", protocol_version: 1, page: "https://example.com/watch?v=1", media: "https://cdn.example/master.m3u8?sig=1", cookies: "sid=abc" };
        expect(fake.done.sent).toEqual([peek]);
        fake.listeners.message({ type: "video-peek", src: "blob:https://example.com/5" }, sender);
        await flush();
        expect(fake.done.sent).toHaveLength(1);
        vi.setSystemTime(Date.now() + 10 * 60_000);
        fake.listeners.message({ type: "video-peek", src: "blob:https://example.com/5" }, sender);
        await flush();
        expect(fake.done.sent).toEqual([peek, peek]);
      } finally {
        vi.useRealTimers();
      }
    });

    it("sends nothing from a private window, nor without the video permission", async () => {
      const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
      await load(fake);
      await flush();
      const tab = { id: 6, url: "https://example.com/v", incognito: true };
      fake.listeners.message({ type: "video-peek", src: "https://cdn.example/v.mp4" }, { id: "zajil", tab, frameId: 0, url: tab.url });
      await flush();
      expect(fake.done.sent).toEqual([]);
      expect(fake.done.cookieReads).toEqual([]);

      const without = fakeChrome({ granted: true, video: false, answer: { ok: true } });
      await load(without);
      await flush();
      const sender = { id: "zajil", tab: { id: 7, url: "https://example.com/w" }, frameId: 0, url: "https://example.com/w" };
      without.listeners.message({ type: "video-peek", src: "https://cdn.example/v.mp4" }, sender);
      await flush();
      expect(without.done.sent).toEqual([]);
    });
  });

  describe("a YouTube playlist, from the button", () => {
    /** Sends `message` from `sender` as the button's script does, and waits for the answer. */
    const press = (fake, message, sender) =>
      new Promise((resolve) => {
        expect(fake.listeners.message(message, sender, resolve)).toBe(true);
      });
    const videos = (fake) => fake.done.sent.filter((m) => m.type === "video");

    it("the playlist page's button sends that page, from the tab's own address, and never the tab's stream", async () => {
      const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
      await load(fake);
      await flush();
      fake.session["streams:4"] = [{ url: "https://cdn.example/now.m3u8", kind: "hls", frame: 0, origin: "https://www.youtube.com" }];
      const sender = { id: "zajil", tab: { id: 4, url: "https://www.youtube.com/playlist?list=PLx" }, frameId: 0, url: "https://www.youtube.com/playlist?list=PLx" };
      expect(await press(fake, { type: "playlist-button" }, sender)).toEqual({ taken: true });
      expect(videos(fake)).toEqual([{ type: "video", protocol_version: 1, page: "https://www.youtube.com/playlist?list=PLx", cookies: "sid=abc" }]);
    });

    it("«The whole playlist» on a video played from one sends the playlist's page, not the video's", async () => {
      const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
      await load(fake);
      await flush();
      const url = "https://www.youtube.com/watch?v=abc&list=PLx&index=2";
      const sender = { id: "zajil", tab: { id: 5, url }, frameId: 0, url };
      expect(await press(fake, { type: "video-button", whole: true, src: "https://cdn.example/v.mp4" }, sender)).toEqual({ taken: true });
      expect(videos(fake)).toEqual([{ type: "video", protocol_version: 1, page: "https://www.youtube.com/playlist?list=PLx", cookies: "sid=abc" }]);
    });

    it("from a private window: no cookies, marked private", async () => {
      const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
      await load(fake);
      await flush();
      const url = "https://www.youtube.com/playlist?list=PLx";
      await press(fake, { type: "playlist-button" }, { id: "zajil", tab: { id: 6, url, incognito: true }, frameId: 0, url });
      expect(videos(fake)).toEqual([{ type: "video", protocol_version: 1, page: url, private: true }]);
      expect(fake.done.cookieReads).toEqual([]);
    });

    it("sends nothing but says so when the tab is on no playlist, or the ask is from a frame", async () => {
      const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
      await load(fake);
      await flush();
      for (const [url, frameId] of /** @type {[string, number][]} */ ([
        ["https://www.youtube.com/watch?v=abc", 0],
        ["https://www.youtube.com/watch?v=abc&list=RDabc", 0],
        ["https://example.com/playlist?list=PLx", 0],
        ["https://www.youtube.com/playlist?list=PLx", 2],
      ])) {
        const answer = await press(fake, { type: "playlist-button" }, { id: "zajil", tab: { id: 7, url }, frameId, url });
        expect(answer, url).toEqual({ taken: false, error: "playlistNotHere" });
        expect(await press(fake, { type: "video-button", whole: true }, { id: "zajil", tab: { id: 7, url }, frameId, url })).toEqual(answer);
      }
      expect(videos(fake)).toEqual([]);
    });
  });

  describe("the button's answers, in Zajil's language", () => {
    const locales = {
      ar: JSON.parse(readFileSync(new URL("../src/_locales/ar/messages.json", import.meta.url), "utf8")),
      en: JSON.parse(readFileSync(new URL("../src/_locales/en/messages.json", import.meta.url), "utf8")),
    };
    beforeEach(() => {
      vi.stubGlobal(
        "fetch",
        vi.fn(async (url) => {
          const lang = /_locales\/(ar|en)\/messages\.json$/.exec(String(url))?.[1];
          return lang ? { ok: true, json: async () => locales[lang] } : { ok: false, status: 404 };
        }),
      );
    });
    afterEach(() => vi.unstubAllGlobals());
    const sender = { id: "zajil", tab: { id: 2, url: "https://example.com/v" }, frameId: 0, url: "https://example.com/v" };
    /** Sends `message` as the button's script does, and waits for the answer. */
    const ask = (fake, message) =>
      new Promise((resolve) => {
        expect(fake.listeners.message(message, sender, resolve)).toBe(true);
      });
    const pings = (fake) => fake.done.sent.filter((m) => m.type === "ping");

    it("its texts are in the language zajil-host says, asked once for asks at once and kept a minute", async () => {
      const fake = fakeChrome({ granted: false, video: true, answer: (m) => (m.type === "ping" ? { ok: true, language: "en" } : { ok: true }) });
      await load(fake);
      await flush();
      const answers = await Promise.all([1, 2, 3].map(() => ask(fake, { type: "video-button-texts" })));
      for (const answer of answers) {
        expect(answer).toEqual({
          lang: "en",
          texts: {
            button: locales.en.videoButton.message,
            working: locales.en.videoButtonWorking.message,
            done: locales.en.videoButtonDone.message,
            failed: locales.en.videoButtonFailed.message,
            playlist: locales.en.playlistButton.message,
            thisVideo: locales.en.videoButtonThis.message,
            wholeList: locales.en.videoButtonWholeList.message,
          },
        });
      }
      expect(pings(fake)).toEqual([{ type: "ping", protocol_version: 1 }]);
      await ask(fake, { type: "video-button-texts" });
      expect(pings(fake)).toHaveLength(1);
      // A minute later, asked again.
      fake.session.zajilLanguage.at -= 61_000;
      await ask(fake, { type: "video-button-texts" });
      expect(pings(fake)).toHaveLength(2);
    });

    it("without Zajil, the browser's language", async () => {
      const fake = fakeChrome({ granted: false, video: true, answer: { ok: true } });
      fake.chrome.runtime.sendNativeMessage = vi.fn(async () => {
        throw new Error("Specified native messaging host not found.");
      });
      await load(fake);
      await flush();
      // The fake's browser says neither: Arabic, Zajil's default.
      expect(await ask(fake, { type: "video-button-texts" })).toMatchObject({ lang: "ar", texts: { button: locales.ar.videoButton.message } });
    });

    it("a press: taken, refused with Zajil's own words, or unreachable in Zajil's language; no notification", async () => {
      let next = { ok: true };
      const fake = fakeChrome({ granted: false, video: true, answer: (m) => (m.type === "ping" ? { ok: true, language: "ar" } : next) });
      await load(fake);
      await flush();
      expect(await ask(fake, { type: "video-button", src: "https://cdn.example/v.mp4" })).toEqual({ taken: true });
      next = { ok: false, error: "زاجِل لا يحمّل هذا الفيديو." };
      expect(await ask(fake, { type: "video-button", src: "https://cdn.example/v.mp4" })).toEqual({ taken: false, error: "زاجِل لا يحمّل هذا الفيديو." });
      next = { ok: false, error: "Zajil did not answer" };
      expect(await ask(fake, { type: "video-button", src: "https://cdn.example/v.mp4" })).toEqual({ taken: false, error: locales.ar.videoNotAnswered.message });
      expect(fake.done.notifications).toEqual([]);
    });

    it("zajil-host's own failure is said in Zajil's language, with nothing of the system (#264)", async () => {
      let next = { ok: false, error: "Zajil's program was not found", code: "not_found" };
      const fake = fakeChrome({ granted: false, video: true, answer: (m) => (m.type === "ping" ? { ok: true, language: "en" } : next) });
      await load(fake);
      await flush();
      expect(await ask(fake, { type: "video-button", src: "https://cdn.example/v.mp4" })).toEqual({ taken: false, error: locales.en.hostNotFound.message });
      next = { ok: false, error: "Zajil is not running", code: "not_running" };
      expect(await ask(fake, { type: "video-button", src: "https://cdn.example/v.mp4" })).toEqual({ taken: false, error: locales.en.hostNotRunning.message });
      // From the menu: the notification says the same, in Zajil's language.
      next = { ok: false, error: "cannot reach Zajil", code: "unreachable" };
      fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v", srcUrl: "https://cdn.example/v.mp4" }, { id: 1, url: "https://example.com/v" });
      await flush();
      expect(fake.done.notifications.at(-1)?.message).toBe(locales.en.hostUnreachable.message);
    });
  });

  it("a message not from the button's script, or not from a tab, is ignored", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    const tab = { id: 1, url: "https://example.com/v" };
    fake.listeners.message({ type: "video-button", src: "https://cdn.example/v.mp4" }, { id: "another-extension", tab });
    fake.listeners.message({ type: "video-button", src: "https://cdn.example/v.mp4" }, { id: "zajil" });
    fake.listeners.message({ type: "download", url: "https://cdn.example/v.mp4" }, { id: "zajil", tab });
    await flush();
    expect(fake.done.sent).toEqual([]);
  });

  it("the menu on a blob video sends the playlist its tab loaded", async () => {
    const fake = fakeChrome({ granted: false, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    // As `note` keeps it: with the origin of the document that loaded it.
    fake.session["streams:8"] = [{ url: "https://cdn.example/m.mpd", kind: "dash", frame: 0, origin: "https://example.com" }];
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v", srcUrl: "blob:https://example.com/2", frameId: 0 }, { id: 8 });
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://example.com/v", media: "https://cdn.example/m.mpd" }]);
    // Another page's playlist (another origin) never goes with this one.
    fake.session["streams:8"] = [{ url: "https://cdn.example/other.mpd", kind: "dash", frame: 0, origin: "https://other.example" }];
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v", srcUrl: "blob:https://example.com/2", frameId: 0 }, { id: 8 });
    await flush();
    expect(fake.done.sent.at(-1)).toEqual({ type: "video", protocol_version: 1, page: "https://example.com/v" });
  });

  it("a link Zajil cannot download is refused before any cookie is read", async () => {
    const fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://user:pw@example.com/a.zip" });
    await flush();
    expect(fake.done.cookieReads).toEqual([]);
    expect(fake.done.sent).toEqual([]);
    expect(fake.done.notifications.at(-1).message).toBe("menuNotALink");
  });

  it("with the permission but no cookie sent, it does not say 'even with the cookies'", async () => {
    const fake = fakeChrome({ granted: true, answer: forbidden, jar: () => [] });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(fake.done.sent[0].cookies).toBeUndefined();
    expect(fake.done.notifications.at(-1).message).toBe("menuSignInNoCookies");
  });

  it("a notice closed without a choice is forgotten", async () => {
    const fake = fakeChrome({ granted: false, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(Object.keys(fake.session)).toEqual(["notice:n1"]);
    fake.listeners.closed("n1");
    await flush();
    expect(Object.keys(fake.session)).toEqual([]);
  });

  it("in Firefox (no notification buttons): the notice is shown without them, and clicking it opens the page", async () => {
    const fake = fakeChrome({ granted: false, buttons: false, answer: forbidden });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" });
    await flush();
    expect(fake.done.notifications.at(-1).buttons).toBeUndefined();
    fake.listeners.clicked("n1");
    await flush();
    expect(fake.done.tabs).toEqual(["chrome-extension://zajil/welcome.html"]);
  });
  it("a link clicked with Zajil's keys goes to Zajil as from its menu, its page the browser's word (the frame's)", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    fake.local.settings = { useKeys: true };
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/" }, frameId: 2, url: "https://files.example/list" };
    fake.listeners.message({ type: "click-link", href: "https://files.example/a.zip" }, sender);
    await flush();
    expect(fake.done.sent).toHaveLength(1);
    expect(fake.done.sent[0]).toMatchObject({ type: "download", url: "https://files.example/a.zip", referrer: "https://files.example/list", cookies: "sid=abc" });
    // Not a web link: refused before any cookie is read, as from the menu.
    const reads = fake.done.cookieReads.length;
    fake.listeners.message({ type: "click-link", href: "javascript:alert(1)" }, sender);
    await flush();
    expect(fake.done.sent).toHaveLength(1);
    expect(fake.done.cookieReads).toHaveLength(reads);
    expect(fake.done.notifications.at(-1).message).toBe("menuNotALink");
    // From a private window: no cookies, marked private.
    fake.listeners.message({ type: "click-link", href: "https://files.example/b.zip" }, { ...sender, tab: { id: 5, url: "https://example.com/", incognito: true } });
    await flush();
    expect(fake.done.sent[1]).toMatchObject({ url: "https://files.example/b.zip", private: true });
    expect(fake.done.sent[1].cookies).toBeUndefined();
  });

  it("the keys do nothing without 'Allow Zajil on sites' (a script left from before), nor from another extension or no tab", async () => {
    const fake = fakeChrome({ granted: true, video: false, answer: { ok: true } });
    await load(fake);
    await flush();
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/" }, frameId: 0, url: "https://example.com/" };
    fake.listeners.message({ type: "click-link", href: "https://example.com/a.zip" }, sender);
    fake.have.add("scripting");
    fake.have.add("webRequest");
    fake.listeners.message({ type: "click-link", href: "https://example.com/a.zip" }, { ...sender, id: "other" });
    fake.listeners.message({ type: "click-link", href: "https://example.com/a.zip" }, { id: "zajil", url: "https://example.com/" });
    await flush();
    expect(fake.done.sent).toEqual([]);
  });

  it("a link clicked with the browser's keys stays the browser's: Zajil does not take its download, once", async () => {
    const fake = fakeChrome({ granted: false, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/" }, frameId: 0, url: "https://example.com/" };
    fake.listeners.message({ type: "click-browser", href: "https://example.com/a.zip" }, sender);
    await flush();
    const item = (id, url) => ({ id, url, finalUrl: url, state: "in_progress" });
    fake.listeners.filename(item(1, "https://example.com/a.zip"), () => {});
    await flush();
    expect(fake.done.sent).toEqual([]);
    // Another link is taken; and the same one, clicked again without the keys.
    fake.listeners.filename(item(2, "https://example.com/b.zip"), () => {});
    fake.listeners.filename(item(3, "https://example.com/a.zip"), () => {});
    await flush();
    expect(fake.done.sent.map((m) => m.url)).toEqual(["https://example.com/b.zip", "https://example.com/a.zip"]);
    // Noted long ago: no longer the browser's.
    fake.session.leave = [{ url: "https://example.com/c.zip", until: Date.now() - 1 }];
    fake.listeners.filename(item(4, "https://example.com/c.zip"), () => {});
    await flush();
    expect(fake.done.sent.map((m) => m.url)).toContain("https://example.com/c.zip");
    expect(fake.session.leave).toEqual([]);
  });

  it("a click left to the browser and its download arriving together: Zajil does not take it; two clicks at once are both kept (review of #169)", async () => {
    const fake = fakeChrome({ granted: false, video: false, answer: { ok: true } });
    await load(fake);
    await flush();
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/" }, frameId: 0, url: "https://example.com/" };
    const item = (id, url) => ({ id, url, finalUrl: url, state: "in_progress" });
    // The same tick, as when the background wakes for both; and without the
    // permission, which leaving a link to the browser does not need.
    fake.listeners.message({ type: "click-browser", href: "https://example.com/a.zip" }, sender);
    fake.listeners.filename(item(1, "https://example.com/a.zip"), () => {});
    fake.listeners.message({ type: "click-browser", href: "https://example.com/b.zip" }, sender);
    fake.listeners.message({ type: "click-browser", href: "https://example.com/c.zip" }, sender);
    await flush();
    expect(fake.done.sent).toEqual([]);
    expect(fake.session.leave.map((l) => l.url)).toEqual(["https://example.com/b.zip", "https://example.com/c.zip"]);
  });

  it("nothing of a private window is noted, nor what is not a web link", async () => {
    const fake = fakeChrome({ granted: false, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    const tab = { id: 4, url: "https://example.com/", incognito: true };
    fake.listeners.message({ type: "click-browser", href: "https://example.com/a.zip" }, { id: "zajil", tab, frameId: 0, url: tab.url });
    fake.listeners.message({ type: "click-browser", href: "javascript:alert(1)" }, { id: "zajil", tab: { ...tab, incognito: false }, frameId: 0, url: tab.url });
    fake.listeners.message({ type: "click-browser", href: "https://example.com/" + "x".repeat(9000) }, { id: "zajil", tab: { ...tab, incognito: false }, frameId: 0, url: tab.url });
    await flush();
    expect(fake.session.leave).toBeUndefined();
  });

  it("tells the keys' scripts in open tabs whether the permission is there", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    expect(fake.local.clickKeysOn).toBe(true);
    fake.have.delete("scripting");
    fake.listeners.permissionsRemoved({ permissions: ["scripting"] });
    await flush();
    expect(fake.local.clickKeysOn).toBe(false);
  });

  it("the keys are off until the user turns them on: a click sent by a script is not taken (owner's decision)", async () => {
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true } });
    await load(fake);
    await flush();
    const sender = { id: "zajil", tab: { id: 4, url: "https://example.com/" }, frameId: 0, url: "https://example.com/" };
    fake.listeners.message({ type: "click-link", href: "https://example.com/a.zip" }, sender);
    await flush();
    expect(fake.done.sent).toEqual([]);
  });

  it("A: a link sent with the keys that Zajil finds to be a web page opens in a tab behind, as Ctrl+click would; from the menu it is said, as before", async () => {
    const page = { protocol_version: 1, ok: false, error: "Zajil cannot download it: the server sent a web page (a sign-in page?), not the file" };
    const fake = fakeChrome({ granted: true, video: true, answer: page });
    await load(fake);
    await flush();
    fake.local.settings = { useKeys: true };
    const tab = { id: 4, index: 2, windowId: 7, url: "https://news.example/" };
    fake.listeners.message({ type: "click-link", href: "https://news.example/story" }, { id: "zajil", tab, frameId: 0, url: tab.url });
    await flush();
    expect(fake.done.tabOptions).toEqual([{ url: "https://news.example/story", active: false, windowId: 7, index: 3, openerTabId: 4 }]);
    expect(fake.done.notifications).toEqual([]);
    // In a Firefox container: the same container.
    fake.listeners.message({ type: "click-link", href: "https://news.example/b" }, { id: "zajil", tab: { ...tab, cookieStoreId: "firefox-container-2" }, frameId: 0, url: tab.url });
    await flush();
    expect(fake.done.tabOptions[1].cookieStoreId).toBe("firefox-container-2");
    // From the menu: no tab, Zajil's answer is said.
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://news.example/c", pageUrl: tab.url }, tab);
    await flush();
    expect(fake.done.tabOptions).toHaveLength(2);
    expect(fake.done.notifications).toHaveLength(1);
    // Another refusal from the keys is said, not opened.
    const other = fakeChrome({ granted: true, video: true, answer: forbidden });
    await load(other);
    await flush();
    other.local.settings = { useKeys: true };
    other.listeners.message({ type: "click-link", href: "https://news.example/story" }, { id: "zajil", tab, frameId: 0, url: tab.url });
    await flush();
    expect(other.done.tabOptions).toEqual([]);
    expect(other.done.notifications).toHaveLength(1);
  });

  it("A: the desktop app's answer for a web page is the one the extension knows", async () => {
    const { readFileSync } = await import("node:fs");
    const { isPageRefusal } = await import("../src/capture.js");
    const rust = readFileSync(new URL("../../desktop/src-tauri/src/lib.rs", import.meta.url), "utf8");
    const why = rust.match(/"(the server sent a web page[^"]*)"/)?.[1];
    expect(why).toBeTruthy();
    expect(isPageRefusal({ outcome: "refused", error: `Zajil cannot download it: ${why}` })).toBe(true);
    expect(isPageRefusal({ outcome: "refused", error: "Zajil cannot download it: the server answered 403 Forbidden" })).toBe(false);
  });

  it("a captured download: strict cookies go only from a page of the same site, and with no page, none (#92)", async () => {
    const jar = () => [
      { name: "s", value: "1", sameSite: "strict", domain: "files.example" },
      { name: "l", value: "2", sameSite: "lax", domain: ".files.example" },
    ];
    const fake = fakeChrome({ granted: true, answer: { ok: true }, jar });
    await load(fake);
    const download = async (id, referrer) => {
      const item = { id, url: "https://files.example/a.zip", finalUrl: "https://files.example/a.zip", state: "in_progress" };
      if (referrer !== undefined) item.referrer = referrer;
      fake.listeners.filename(item, () => {});
      await flush();
      return fake.done.sent.at(-1);
    };
    const { answer } = await import("../src/link.js");
    const again = async (ticket) => (await answer(fake.chrome, { type: "cookies_wanted", ask: 1, url: "https://files.example/a.zip", ticket })).cookies;
    // From another site's page: the browser sends no strict cookie, nor Zajil, now or later.
    const other = await download(1, "https://forum.example/thread");
    expect(other.cookies).toBe("l=2");
    expect(await again(other.ticket)).toBe("l=2");
    // From the same site: all of them, now and later.
    const same = await download(2, "https://files.example/list");
    expect(same.cookies).toBe("s=1; l=2");
    expect(await again(same.ticket)).toBe("s=1; l=2");
    // With no page to tell by: none of the strict ones.
    const none = await download(3, undefined);
    expect(none.cookies).toBe("l=2");
    expect(await again(none.ticket)).toBe("l=2");
    const empty = await download(4, "");
    expect(empty.cookies).toBe("l=2");
  });

  it("B: from the keys, strict cookies go only from a page of the same site, as in the browser; and not when Zajil asks again later", async () => {
    const jar = () => [
      { name: "s", value: "1", sameSite: "strict", domain: "files.example" },
      { name: "l", value: "2", sameSite: "lax", domain: ".files.example" },
      { name: "n", value: "3", sameSite: "no_restriction", domain: ".files.example" },
    ];
    const fake = fakeChrome({ granted: true, video: true, answer: { ok: true }, jar });
    await load(fake);
    await flush();
    fake.local.settings = { useKeys: true };
    const click = (page) => fake.listeners.message({ type: "click-link", href: "https://files.example/a.zip" }, { id: "zajil", tab: { id: 4, url: page }, frameId: 0, url: page });
    // Another site's page: no strict cookie.
    click("https://forum.example/thread");
    await flush();
    expect(fake.done.sent[0].cookies).toBe("l=2; n=3");
    // The same site (the cookie's own domain, or under it): all of them.
    click("https://files.example/list");
    click("https://www.files.example/list");
    await flush();
    expect(fake.done.sent[1].cookies).toBe("s=1; l=2; n=3");
    expect(fake.done.sent[2].cookies).toBe("s=1; l=2; n=3");
    // Another scheme is another site; and a look-alike is not under it.
    click("http://files.example/list");
    click("https://evilfiles.example/");
    await flush();
    expect(fake.done.sent[3].cookies).toBe("l=2; n=3");
    expect(fake.done.sent[4].cookies).toBe("l=2; n=3");
    // From the menu, as before: all of them.
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://files.example/a.zip", pageUrl: "https://forum.example/thread" }, { id: 4 });
    await flush();
    expect(fake.done.sent[5].cookies).toBe("s=1; l=2; n=3");
    // Asked again after a restart: the link from another site still gets
    // no strict cookie; the one from the same site does.
    const { answer } = await import("../src/link.js");
    const again = async (ticket) => (await answer(fake.chrome, { type: "cookies_wanted", ask: 1, url: "https://files.example/a.zip", ticket })).cookies;
    expect(await again(fake.done.sent[0].ticket)).toBe("l=2; n=3");
    expect(await again(fake.done.sent[1].ticket)).toBe("s=1; l=2; n=3");
    expect(await again(fake.done.sent[5].ticket)).toBe("s=1; l=2; n=3");
  });
});
