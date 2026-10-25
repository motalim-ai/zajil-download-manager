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

  it("the video's frame's newest goes with it, else the tab's newest, else none", async () => {
    const api = browserWith();
    expect(await streamFor(api, 1, 0)).toBeUndefined();
    await note(api, hls("https://player.example/embed.m3u8", { frameId: 7 }));
    await note(api, hls("https://cdn.example/ad.m3u8", { frameId: 0 }));
    expect(await streamFor(api, 1, 7)).toBe("https://player.example/embed.m3u8");
    expect(await streamFor(api, 1, 0)).toBe("https://cdn.example/ad.m3u8");
    expect(await streamFor(api, 1, 9)).toBe("https://cdn.example/ad.m3u8");
  });
});
