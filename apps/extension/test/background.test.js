import { afterEach, describe, expect, it, vi } from "vitest";

/** A fake `chrome` for the real `background.js`: its listeners are kept to
 * be fired, and what it does is recorded. `granted`: the cookie
 * permission; `buttons`: whether notifications take buttons (Firefox:
 * not); `answer`: what Zajil answers. */
function fakeChrome({ granted, buttons = true, answer, jar = () => [{ name: "sid", value: "abc" }] }) {
  const listeners = {};
  const on = (name) => ({ addListener: (fn) => (listeners[name] = fn) });
  const session = {};
  const done = { tabs: [], notifications: [], downloads: [], sent: [], cleared: [], cookieReads: [] };
  const local = {};
  const chrome = {
    i18n: { getMessage: (key) => key },
    runtime: {
      id: "zajil",
      getURL: (path) => `chrome-extension://zajil/${path}`,
      onInstalled: on("installed"),
      openOptionsPage: async () => {},
      sendNativeMessage: vi.fn(async (_host, message) => {
        done.sent.push(message);
        return answer;
      }),
    },
    permissions: { contains: async () => granted },
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
        get: async (key) => ({ [key]: session[key] }),
        set: async (value) => Object.assign(session, value),
        remove: async (key) => delete session[key],
      },
    },
    tabs: { create: async (tab) => done.tabs.push(tab.url) },
    action: { setBadgeText: async () => {}, onClicked: on("action") },
    contextMenus: { create: () => {}, onClicked: on("menu") },
    downloads: {
      onDeterminingFilename: on("filename"),
      download: async (options) => done.downloads.push(options.url),
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
  return { chrome, listeners, done, session, local };
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
    // Only the download link's cookies are read, not the page's.
    expect(fake.done.cookieReads.map((q) => q.url)).toEqual(["https://example.com/a.zip", "https://example.com/b.zip"]);
  });

  it("from a private window, the menu sends no cookies and reads none", async () => {
    const fake = fakeChrome({ granted: true, answer: { ok: true } });
    await load(fake);
    fake.listeners.menu({ menuItemId: "zajil-download-link", linkUrl: "https://example.com/a.zip" }, { incognito: true });
    await flush();
    expect(fake.done.sent[0].cookies).toBeUndefined();
    expect(fake.done.cookieReads).toEqual([]);
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
