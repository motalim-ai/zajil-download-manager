import { afterEach, describe, expect, it, vi } from "vitest";

/** A fake `chrome` for the real `background.js`: its listeners are kept to
 * be fired, and what it does is recorded. `granted`: the cookie
 * permission; `buttons`: whether notifications take buttons (Firefox:
 * not); `answer`: what Zajil answers. */
function fakeChrome({ granted, video = false, buttons = true, answer, jar = () => [{ name: "sid", value: "abc" }], windows = [], tabs = {} }) {
  const listeners = {};
  const on = (name) => ({ addListener: (fn) => (listeners[name] = fn) });
  const session = {};
  const done = { tabs: [], notifications: [], downloads: [], sent: [], cleared: [], cookieReads: [], ports: [], downloadOptions: [], menus: [], scripts: [], watched: [] };
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
        return answer;
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
          getRegisteredContentScripts: async () => done.scripts,
          registerContentScripts: async (list) => done.scripts.push(...list),
          unregisterContentScripts: async () => done.scripts.splice(0),
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
      create: async (tab) => done.tabs.push(tab.url),
      get: async (id) => tabs[id] ?? { id, incognito: false },
      onRemoved: on("tabRemoved"),
    },
    windows: { onRemoved: on("windowRemoved"), getAll: async () => windows },
    action: { setBadgeText: async () => {}, onClicked: on("action") },
    contextMenus: { create: (item) => done.menus.push(item), onClicked: on("menu") },
    downloads: {
      onDeterminingFilename: on("filename"),
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
    fake.session["streams:8"] = [{ url: "https://cdn.example/m.mpd", kind: "dash", frame: 0 }];
    fake.listeners.menu({ menuItemId: "zajil-download-video", pageUrl: "https://example.com/v", srcUrl: "blob:https://example.com/2", frameId: 0 }, { id: 8 });
    await flush();
    expect(fake.done.sent).toEqual([{ type: "video", protocol_version: 1, page: "https://example.com/v", media: "https://cdn.example/m.mpd" }]);
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
});
