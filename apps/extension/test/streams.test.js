import { describe, expect, it } from "vitest";
import { MAX_STREAMS, forget, forgetAll, kindOf, note, streamFor, streamsOf } from "../src/streams.js";

/** A fake browser: `storage.session` only (memory), and its tabs. Anything
 * written elsewhere would throw. */
function browserWith(tabs = {}) {
  const session = {};
  return {
    session,
    storage: {
      session: {
        get: async (key) => (key === null ? { ...session } : { [key]: session[key] }),
        set: async (value) => Object.assign(session, value),
        remove: async (key) => [key].flat().forEach((k) => delete session[k]),
      },
    },
    tabs: { get: async (id) => tabs[id] ?? { id, incognito: false } },
  };
}

const hls = (url, extra = {}) => ({
  url,
  tabId: 1,
  frameId: 0,
  type: "xmlhttprequest",
  responseHeaders: [{ name: "Content-Type", value: "application/vnd.apple.mpegurl; charset=utf-8" }],
  ...extra,
});

describe("kindOf", () => {
  it("knows HLS and DASH by their type, else by their path", () => {
    expect(kindOf(hls("https://cdn.example/a"))).toBe("hls");
    expect(kindOf({ url: "https://cdn.example/a", responseHeaders: [{ name: "content-type", value: "application/dash+xml" }] })).toBe("dash");
    expect(kindOf({ url: "https://cdn.example/live/index.M3U8?token=1", responseHeaders: [{ name: "Content-Type", value: "text/plain" }] })).toBe("hls");
    expect(kindOf({ url: "https://cdn.example/v/manifest.mpd" })).toBe("dash");
  });

  it("leaves everything else: segments, pages, scripts", () => {
    for (const url of ["https://cdn.example/seg-1.ts", "https://cdn.example/v.mp4", "https://example.com/m3u8.html", "not a url"]) {
      expect(kindOf({ url, responseHeaders: [{ name: "Content-Type", value: "video/mp2t" }] }), url).toBeNull();
    }
  });
});

describe("the playlists a tab loads", () => {
  it("are kept for their tab only, newest last, once each", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/a.m3u8"));
    await note(api, hls("https://cdn.example/b.m3u8"));
    await note(api, hls("https://cdn.example/a.m3u8"));
    await note(api, hls("https://cdn.example/other.m3u8", { tabId: 2 }));
    expect((await streamsOf(api, 1)).map((s) => s.url)).toEqual(["https://cdn.example/b.m3u8", "https://cdn.example/a.m3u8"]);
    expect((await streamsOf(api, 2)).map((s) => s.url)).toEqual(["https://cdn.example/other.m3u8"]);
    // In memory only: `storage.session`.
    expect(Object.keys(api.session).sort()).toEqual(["streams:1", "streams:2"]);
  });

  it("at most MAX_STREAMS, also when they come all at once", async () => {
    const api = browserWith();
    await Promise.all(Array.from({ length: MAX_STREAMS + 5 }, (_, i) => note(api, hls(`https://cdn.example/${i}.m3u8`))));
    const kept = await streamsOf(api, 1);
    expect(kept).toHaveLength(MAX_STREAMS);
    expect(kept.at(-1)?.url).toBe(`https://cdn.example/${MAX_STREAMS + 4}.m3u8`);
  });

  it("are forgotten when the tab loads another page, or closes", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/a.m3u8"));
    await note(api, { url: "https://example.com/next", tabId: 1, type: "main_frame" });
    expect(await streamsOf(api, 1)).toEqual([]);
    await note(api, hls("https://cdn.example/b.m3u8"));
    await forget(api, 1);
    expect(await streamsOf(api, 1)).toEqual([]);
    expect(api.session).toEqual({});
  });

  it("a late answer about the old page's tab never brings its playlist back after the new page (review)", async () => {
    const api = browserWith();
    let answer;
    api.tabs.get = () => new Promise((resolve) => (answer = resolve));
    const old = note(api, hls("https://old.example/a.m3u8"));
    const next = note(api, { url: "https://example.com/next", tabId: 1, type: "main_frame" });
    await new Promise((resolve) => setTimeout(resolve, 10));
    answer({ id: 1, incognito: false });
    await Promise.all([old, next]);
    expect(await streamsOf(api, 1)).toEqual([]);
  });

  it("all are forgotten at once, and nothing else in the session", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/a.m3u8"));
    await note(api, hls("https://cdn.example/b.m3u8", { tabId: 2 }));
    api.session.privateSent = true;
    await forgetAll(api);
    expect(api.session).toEqual({ privateSent: true });
  });

  it("nothing from a private window: Firefox says so on the request, Chrome is asked", async () => {
    const api = browserWith({ 3: { id: 3, incognito: true } });
    await note(api, hls("https://cdn.example/a.m3u8", { incognito: true }));
    await note(api, hls("https://cdn.example/a.m3u8", { tabId: 3 }));
    expect(api.session).toEqual({});
  });

  it("nothing that is not a tab's, or not a link Zajil can download", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/a.m3u8", { tabId: -1 }));
    await note(api, hls("https://user:pw@cdn.example/a.m3u8"));
    await note(api, hls(`https://cdn.example/${"a".repeat(9000)}.m3u8`));
    expect(api.session).toEqual({});
  });

  it("a video gets the newest playlist of its own document: by its id, else its frame and origin; never another frame's", async () => {
    const api = browserWith();
    expect(await streamFor(api, 1, { frame: 0, origin: "https://site.example" })).toBeUndefined();
    await note(api, hls("https://player.example/embed.m3u8", { frameId: 7, initiator: "https://player.example" }));
    await note(api, hls("https://cdn.example/ad.m3u8", { frameId: 9, initiator: "https://ads.example" }));
    await note(api, hls("https://cdn.example/main.m3u8", { frameId: 0, documentUrl: "https://site.example/watch" }));
    expect(await streamFor(api, 1, { frame: 7, origin: "https://player.example" })).toBe("https://player.example/embed.m3u8");
    expect(await streamFor(api, 1, { frame: 0, origin: "https://site.example" })).toBe("https://cdn.example/main.m3u8");
    // An ad's frame never lends its playlist to the page, nor the reverse.
    expect(await streamFor(api, 1, { frame: 0, origin: "https://ads.example" })).toBeUndefined();
    expect(await streamFor(api, 1, { frame: 9, origin: "https://site.example" })).toBeUndefined();
    expect(await streamFor(api, 1, { frame: 3, origin: "https://site.example" })).toBeUndefined();
    // Nothing known of either side: nothing goes.
    expect(await streamFor(api, 1, { frame: 0 })).toBeUndefined();
  });

  it("a page restored by Back gets none of the next page's playlists (Chrome's documentId)", async () => {
    const api = browserWith();
    // Page A, then B in the same tab; Back brings A from the cache with no
    // request at all, so B's playlist is still noted.
    await note(api, { url: "https://b.example/", tabId: 1, frameId: 0, type: "main_frame" });
    await note(api, hls("https://cdn.example/b.m3u8", { initiator: "https://site.example", documentId: "doc-B" }));
    expect(await streamFor(api, 1, { frame: 0, doc: "doc-A", origin: "https://site.example" })).toBeUndefined();
    expect(await streamFor(api, 1, { frame: 0, doc: "doc-B", origin: "https://site.example" })).toBe("https://cdn.example/b.m3u8");
  });

  it("a prerendered page does not forget the current page's playlists; a real new page does", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/a.m3u8", { initiator: "https://site.example" }));
    await note(api, { url: "https://site.example/next", tabId: 1, frameId: 0, type: "main_frame", documentLifecycle: "prerender" });
    expect(await streamsOf(api, 1)).toHaveLength(1);
    await note(api, { url: "https://site.example/next", tabId: 1, frameId: 0, type: "main_frame", documentLifecycle: "active" });
    expect(await streamsOf(api, 1)).toEqual([]);
  });

  it("an answer that is not a success is no playlist", async () => {
    const api = browserWith();
    await note(api, hls("https://cdn.example/missing.m3u8", { statusCode: 404 }));
    await note(api, hls("https://cdn.example/moved.m3u8", { statusCode: 302 }));
    expect(api.session).toEqual({});
    await note(api, hls("https://cdn.example/ok.m3u8", { statusCode: 200 }));
    expect(await streamsOf(api, 1)).toHaveLength(1);
  });
});
