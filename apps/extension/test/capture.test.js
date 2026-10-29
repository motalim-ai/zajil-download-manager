import { describe, expect, it, vi } from "vitest";
import { readFileSync } from "node:fs";
import {
  HOST,
  HOST_PROBLEMS,
  MAX_URL,
  NO_ANSWER,
  downloadMessage,
  extensionOf,
  menuMessage,
  playlistPage,
  send,
  shouldCapture,
  takeOver,
  samePost,
  videoMessage,
} from "../src/capture.js";
import { DECIDE_MS, onNewDownload, pausesSafely, withDownloadUiHidden } from "../src/compat.js";
import { defaults } from "../src/settings.js";

const item = (over = {}) => ({
  id: 7,
  url: "https://example.com/files/setup.exe",
  finalUrl: "https://cdn.example.com/files/setup.exe",
  referrer: "https://example.com/download",
  filename: "",
  state: "in_progress",
  ...over,
});

/** A fake `browser` with what the extension calls, recording the calls. */
function fakeApi({ answer, nativeError, pauseError } = {}) {
  const calls = [];
  const record = (name) => vi.fn(async (...args) => {
    calls.push([name, ...args]);
    if (name === "pause" && pauseError) throw new Error(pauseError);
    if (name === "native") {
      if (nativeError) throw new Error(nativeError);
      return answer;
    }
  });
  return {
    calls,
    downloads: { pause: record("pause"), resume: record("resume"), cancel: record("cancel"), erase: record("erase") },
    runtime: { sendNativeMessage: record("native") },
  };
}

describe("which downloads go to Zajil", () => {
  it("captures a listed type, from the browser's name or the URL", () => {
    const s = defaults();
    expect(shouldCapture(item(), s)).toBe(true);
    expect(shouldCapture(item({ filename: "C:\\Users\\u\\Downloads\\Movie.MKV", finalUrl: "https://x.test/get?id=3" }), s)).toBe(true);
    expect(shouldCapture(item({ filename: "/home/u/Downloads/report.pdf" }), s)).toBe(true);
    expect(extensionOf({ url: "https://x.test/a/%D8%B2%D8%A7%D8%AC%D9%84.ZIP?x=1#y" })).toBe("zip");
  });

  it("leaves everything else in the browser", () => {
    const s = defaults();
    for (const over of [
      { finalUrl: "https://example.com/page.html" },
      { finalUrl: "https://example.com/noextension" },
      { finalUrl: "blob:https://example.com/1234", url: "blob:https://example.com/1234" },
      { finalUrl: "data:application/zip;base64,AAAA", url: "data:application/zip;base64,AAAA" },
      { finalUrl: "ftp://example.com/a.zip", url: "ftp://example.com/a.zip" },
      { finalUrl: "https://user:pw@example.com/a.zip" },
      { finalUrl: `https://example.com/${"a".repeat(MAX_URL)}.zip` },
      { state: "complete" },
      { byExtensionId: "zajil-self" },
      { incognito: true },
      { finalUrl: "https://example.com/\u202epiz.exe" },
    ]) {
      expect(shouldCapture(item(over), s, "zajil-self"), JSON.stringify(over)).toBe(false);
    }
    expect(shouldCapture(item(), { ...s, capture: false })).toBe(false);
    expect(shouldCapture(item(), { ...s, extensions: ["zip"] })).toBe(false);
  });

  it("takes a file of any type larger than the size set, when the browser knows its size", () => {
    const s = defaults();
    const MB = 1024 * 1024;
    const page = { finalUrl: "https://example.com/get?id=3", url: "https://example.com/get?id=3", filename: "" };
    expect(shouldCapture(item({ ...page, totalBytes: 5 * MB + 1 }), s)).toBe(true);
    expect(shouldCapture(item({ ...page, fileSize: 40 * MB, totalBytes: 0 }), s)).toBe(true);
    // At the size or under it, or not known (Chrome 0, Firefox -1): the
    // type decides, as before.
    for (const size of [{ totalBytes: 5 * MB }, { totalBytes: 0 }, { totalBytes: -1 }, {}]) {
      expect(shouldCapture(item({ ...page, ...size }), s), JSON.stringify(size)).toBe(false);
    }
    expect(shouldCapture(item({ totalBytes: 10 }), s)).toBe(true);
    expect(shouldCapture(item({ ...page, totalBytes: 50 * MB }), { ...s, bigFiles: false })).toBe(false);
    expect(shouldCapture(item({ ...page, totalBytes: 50 * MB }), { ...s, bigFileMb: 100 })).toBe(false);
    // Every other rule still holds.
    for (const over of [{ incognito: true }, { state: "complete" }, { finalUrl: "blob:https://example.com/1", url: "blob:https://example.com/1" }]) {
      expect(shouldCapture(item({ ...page, totalBytes: 50 * MB, ...over }), s), JSON.stringify(over)).toBe(false);
    }
    expect(shouldCapture(item({ ...page, totalBytes: 50 * MB }), { ...s, excludedSites: ["example.com"] })).toBe(false);
    expect(shouldCapture(item({ ...page, totalBytes: 50 * MB }), { ...s, capture: false })).toBe(false);
  });

  it("captures the new types too: split RAR parts and more video", () => {
    const s = defaults();
    for (const name of ["movie.part.r00", "movie.r19", "clip.3gp", "film.rmvb", "stream.m2ts", "update.msu", "firmware.bin"]) {
      expect(shouldCapture(item({ finalUrl: `https://example.com/${name}` }), s), name).toBe(true);
    }
    expect(shouldCapture(item({ finalUrl: "https://example.com/movie.r20" }), s)).toBe(false);
    // Not `ts` (owner's decision, 2026-10-27: TypeScript files have it too);
    // a large video with it goes by its size.
    expect(shouldCapture(item({ finalUrl: "https://example.com/app.ts", totalBytes: 2048 }), s)).toBe(false);
    expect(shouldCapture(item({ finalUrl: "https://example.com/live.ts", totalBytes: 50 * 1024 * 1024 }), s)).toBe(true);
  });

  it("respects excluded sites, for the file and for the page it came from", () => {
    const s = { ...defaults(), excludedSites: ["example.com"] };
    expect(shouldCapture(item(), s)).toBe(false);
    expect(shouldCapture(item({ referrer: "https://other.test/", finalUrl: "https://dl.example.com/a.zip" }), s)).toBe(false);
    expect(shouldCapture(item({ referrer: "https://other.test/", finalUrl: "https://badexample.com/a.zip", url: "https://badexample.com/a.zip" }), s)).toBe(true);
    // An excluded link that redirects elsewhere, with no page: still excluded.
    expect(shouldCapture(item({ url: "https://example.com/get/a.zip", finalUrl: "https://cdn.other.test/a.zip", referrer: "" }), s)).toBe(false);
  });
});

describe("the message zajil-host accepts", () => {
  it("has the URL, and the page and a plain file name when the host would take them", () => {
    expect(downloadMessage("https://example.com/a.zip", { referrer: "https://example.com/", filename: "/tmp/dl/a.zip" })).toEqual({
      type: "download",
      protocol_version: 1,
      url: "https://example.com/a.zip",
      referrer: "https://example.com/",
      filename: "a.zip",
    });
  });

  it("leaves out what the host would refuse, so the download is not refused for it", () => {
    for (const extra of [
      { referrer: "chrome://newtab/" },
      { referrer: "https://user:pw@example.com/" },
      { filename: "bad\nname.zip" },
      { filename: "\u0007.zip" },
      { filename: "a\u0085b.zip" },
      { filename: "a\u009bb.zip" },
      { filename: ".." },
      { filename: `${"ز".repeat(128)}.zip` },
    ]) {
      expect(Object.keys(downloadMessage("https://example.com/a.zip", extra)).sort(), JSON.stringify(extra)).toEqual(["protocol_version", "type", "url"]);
    }
  });
});

describe("Download with Zajil on a link", () => {
  it("sends a web link with its page, and nothing else", () => {
    expect(menuMessage({ linkUrl: "https://example.com/a.zip", pageUrl: "https://example.com/" })).toEqual(
      downloadMessage("https://example.com/a.zip", { referrer: "https://example.com/" }),
    );
    for (const linkUrl of [undefined, "javascript:alert(document.cookie)", "data:text/plain,x", "ftp://example.com/a.zip", "https://u:p@example.com/a.zip", `https://example.com/${"a".repeat(MAX_URL)}`, "https://example.com/\u202e.zip", "https://example.com/a\u2028.zip", "https://example.com/a\u2029.zip"]) {
      expect(menuMessage({ linkUrl, pageUrl: "https://example.com/" }), String(linkUrl)).toBeNull();
    }
  });
});

/** A fake that behaves as Firefox: pausing a download with no data yet
 * cancels it, and resuming it then fails (review of #70). */
function firefoxApi(answer) {
  const api = fakeApi({ answer });
  let cancelled = false;
  api.downloads.pause = vi.fn(async () => {
    api.calls.push(["pause"]);
    cancelled = true;
  });
  api.downloads.resume = vi.fn(async () => {
    api.calls.push(["resume"]);
    if (cancelled) throw new Error("Download is not resumable");
  });
  api.downloads.onCreated = { addListener() {} };
  return api;
}

describe("handing a download over, in Firefox", () => {
  it("is told apart from Chrome", () => {
    expect(pausesSafely(firefoxApi())).toBe(false);
    expect(pausesSafely({ downloads: { onDeterminingFilename: {} } })).toBe(true);
  });

  it("refused, unreachable or unanswered: the browser's download is never touched", async () => {
    for (const answer of [{ ok: false, error: "refused" }, { ok: false, error: `${NO_ANSWER[0]} within 10 s` }]) {
      const api = firefoxApi(answer);
      const outcome = (await takeOver(api, item(), { pause: pausesSafely(api) })).outcome;
      expect(api.calls.map((c) => c[0]), outcome).toEqual(["native"]);
    }
    const api = firefoxApi();
    api.runtime.sendNativeMessage = vi.fn(async () => {
      api.calls.push(["native"]);
      throw new Error("No such native application");
    });
    expect((await takeOver(api, item(), { pause: false })).outcome).toBe("unreachable");
    expect(api.calls.map((c) => c[0])).toEqual(["native"]);
  });

  it("taken: cancelled and removed only then", async () => {
    const api = firefoxApi({ protocol_version: 1, ok: true });
    api.downloads.search = vi.fn(async () => [{ id: 7, state: "in_progress" }]);
    expect((await takeOver(api, item(), { pause: pausesSafely(api) })).outcome).toBe("taken");
    expect(api.calls.map((c) => c[0])).toEqual(["native", "cancel", "erase"]);
  });

  it("taken after the browser finished it: the browser's file and line stay", async () => {
    const api = firefoxApi({ protocol_version: 1, ok: true });
    api.downloads.search = vi.fn(async () => [{ id: 7, state: "complete" }]);
    expect(await takeOver(api, item(), { pause: false })).toEqual({ outcome: "taken", alsoInBrowser: true });
    expect(api.calls.map((c) => c[0])).toEqual(["native"]);
  });

  it("pausing as in Chrome would lose it: the fake shows why the rule exists", async () => {
    const api = firefoxApi({ ok: false, error: "refused" });
    await takeOver(api, item(), { pause: true });
    await expect(api.downloads.resume(7)).rejects.toThrow("not resumable");
  });
});

describe("handing a download over, in Chrome and Edge", () => {
  it("taken by Zajil: the browser's download is paused, then cancelled and removed", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: true } });
    expect(await takeOver(api, item(), { pause: true })).toEqual({ outcome: "taken" });
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "cancel", "erase"]);
    expect(api.calls[1][1]).toBe(HOST);
    expect(api.calls[1][2].url).toBe("https://cdn.example.com/files/setup.exe");
  });

  it("refused by Zajil: the browser goes on with it", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: false, error: "the URL has control characters" } });
    expect((await takeOver(api, item(), { pause: true })).outcome).toBe("refused");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "resume"]);
  });

  it("no host (Zajil not installed): the browser goes on with it", async () => {
    const api = fakeApi({ nativeError: "Specified native messaging host not found." });
    expect((await takeOver(api, item(), { pause: true })).outcome).toBe("unreachable");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "resume"]);
  });

  it("no answer in time: it stays paused, since Zajil may have it (#64)", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: false, error: `${NO_ANSWER[0]} within 10 s` } });
    expect((await takeOver(api, item(), { pause: true })).outcome).toBe("unknown");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native"]);
  });

  it("already finished in the browser: nothing is sent", async () => {
    const api = fakeApi({ pauseError: "Download must be in progress" });
    expect(await takeOver(api, item(), { pause: true })).toBeNull();
    expect(api.calls.map((c) => c[0])).toEqual(["pause"]);
  });

  it("an answer that came back broken counts as unknown, not refused", async () => {
    for (const error of ["Zajil did not answer: connection reset", "Zajil's answer is not valid: EOF"]) {
      const api = fakeApi({ answer: { protocol_version: 1, ok: false, error } });
      expect((await takeOver(api, item(), { pause: true })).outcome, error).toBe("unknown");
      expect(api.calls.map((c) => c[0])).toEqual(["pause", "native"]);
    }
  });

  it("zajil-host's own failures come with their code; only no answer is unknown", async () => {
    const answer = (code, error) => fakeApi({ answer: { protocol_version: 1, ok: false, error, code } });
    expect(await send(answer("no_answer", "Zajil did not answer"), {})).toEqual({ outcome: "unknown", error: "Zajil did not answer", code: "no_answer" });
    for (const code of ["not_running", "not_found", "unreachable"]) {
      expect(await send(answer(code, "x"), {}), code).toEqual({ outcome: "refused", error: "x", code });
    }
    // A code it does not know is not passed on: the text decides, as before.
    expect(await send(answer("weird", "Zajil did not answer"), {})).toEqual({ outcome: "unknown", error: "Zajil did not answer" });
    // Every code has its text in both languages.
    for (const lang of ["ar", "en"]) {
      const texts = JSON.parse(readFileSync(new URL(`../src/_locales/${lang}/messages.json`, import.meta.url), "utf8"));
      for (const key of Object.values(HOST_PROBLEMS)) expect(texts[key]?.message, `${lang} ${key}`).toBeTruthy();
    }
  });

  it("a malformed answer counts as refused", async () => {
    expect((await send(fakeApi({ answer: undefined }), {})).outcome).toBe("refused");
    expect((await send(fakeApi({ answer: { ok: "yes" } }), {})).outcome).toBe("refused");
  });
});

describe("videoMessage (M6)", () => {
  it("sends the frame's page for an embedded player, else the tab's, and an http(s) video address", () => {
    expect(videoMessage({ pageUrl: "https://blog.example/post", frameUrl: "https://player.example/embed/1", srcUrl: "https://cdn.example/v.mp4" })).toEqual({
      type: "video",
      protocol_version: 1,
      page: "https://player.example/embed/1",
      media: "https://cdn.example/v.mp4",
    });
    expect(videoMessage({ pageUrl: "https://example.com/v", frameUrl: "about:blank" })).toEqual({ type: "video", protocol_version: 1, page: "https://example.com/v" });
  });

  it("sends a feed's video as its post's page, only of the same site (X's timeline)", () => {
    const post = samePost("https://x.com/someone/status/1", "https://x.com/motalim_ai");
    expect(post).toBe("https://x.com/someone/status/1");
    expect(videoMessage({ pageUrl: "https://x.com/motalim_ai", postUrl: post, srcUrl: "blob:https://x.com/1" })).toEqual({
      type: "video",
      protocol_version: 1,
      page: "https://x.com/someone/status/1",
    });
    expect(samePost("https://elsewhere.example/status/1", "https://x.com/home")).toBeUndefined();
    expect(samePost("javascript:alert(1)", "https://x.com/home")).toBeUndefined();
    expect(samePost(42, "https://x.com/home")).toBeUndefined();
  });

  it("leaves out an address only the page can read, and refuses a page Zajil cannot", () => {
    for (const srcUrl of ["blob:https://example.com/1", "data:video/mp4;base64,AAAA", "https://u:p@example.com/v.mp4", undefined]) {
      expect(videoMessage({ pageUrl: "https://example.com/v", srcUrl })?.media, String(srcUrl)).toBeUndefined();
    }
    for (const pageUrl of ["file:///v.html", "chrome://newtab", "https://example.com/\u202ev", `https://example.com/${"a".repeat(MAX_URL)}`, undefined]) {
      expect(videoMessage({ pageUrl }), String(pageUrl)).toBeNull();
    }
  });

  it("sends the cookies, but never a private window's", () => {
    expect(videoMessage({ pageUrl: "https://example.com/v" }, { cookies: "a=1" })).toMatchObject({ cookies: "a=1" });
    const sent = videoMessage({ pageUrl: "https://example.com/v" }, { cookies: "a=1", private: true });
    expect(sent).toMatchObject({ private: true });
    expect(sent?.cookies).toBeUndefined();
  });
});

describe("the browser's download UI while Zajil takes a download", () => {
  /** Chrome's `downloads.setUiOptions`, recording what it was told. */
  function uiApi() {
    const told = [];
    return { told, downloads: { setUiOptions: vi.fn(async (options) => void told.push(options.enabled)) } };
  }

  it("is hidden while the download is handed over, and comes back after", async () => {
    const browserApi = uiApi();
    const seen = await withDownloadUiHidden(browserApi, async () => [...browserApi.told]);
    expect(seen).toEqual([false]);
    expect(browserApi.told).toEqual([false, true]);
  });

  it("comes back when the hand-over fails", async () => {
    const browserApi = uiApi();
    await expect(withDownloadUiHidden(browserApi, async () => Promise.reject(new Error("no")))).rejects.toThrow("no");
    expect(browserApi.told).toEqual([false, true]);
  });

  it("stays hidden until the last of two hand-overs at once is done", async () => {
    const browserApi = uiApi();
    /** @type {() => void} */
    let finishFirst = () => {};
    const first = withDownloadUiHidden(browserApi, () => new Promise((done) => (finishFirst = () => done(1))));
    const second = withDownloadUiHidden(browserApi, async () => 2);
    expect(await second).toBe(2);
    expect(browserApi.told).toEqual([false]);
    finishFirst();
    expect(await first).toBe(1);
    expect(browserApi.told).toEqual([false, true]);
  });

  it("is left alone in Firefox, which has no such call", async () => {
    const browserApi = { downloads: {} };
    expect(await withDownloadUiHidden(browserApi, async () => "ran")).toBe("ran");
  });

  it("a refused call does not stop the hand-over", async () => {
    const browserApi = { downloads: { setUiOptions: vi.fn(async () => Promise.reject(new Error("no permission"))) } };
    expect(await withDownloadUiHidden(browserApi, async () => "ran")).toBe("ran");
    expect(browserApi.downloads.setUiOptions).toHaveBeenCalledTimes(2);
  });
});

describe("a new download in Chrome waits for the extension to decide", () => {
  /** Chrome's `onDeterminingFilename`, with the listener the extension added. */
  function chromeApi() {
    /** @type {any} */
    let listener;
    return { fire: (/** @type {any} */ item, /** @type {() => void} */ suggest) => listener(item, suggest), downloads: { onDeterminingFilename: { addListener: (/** @type {any} */ l) => (listener = l) } } };
  }

  it("settles once the handler is ready, and only once", async () => {
    const browser = chromeApi();
    /** @type {() => void} */
    let goOn = () => {};
    onNewDownload(browser, (_item, ready) => new Promise((done) => (goOn = () => { ready(); done(undefined); })));
    const suggest = vi.fn();
    expect(browser.fire({ id: 1 }, suggest)).toBe(true);
    await new Promise((r) => setTimeout(r, 10));
    expect(suggest).not.toHaveBeenCalled();
    goOn();
    await new Promise((r) => setTimeout(r, 10));
    expect(suggest).toHaveBeenCalledTimes(1);
  });

  it("settles when the handler fails, or takes too long", async () => {
    const failing = chromeApi();
    onNewDownload(failing, async () => {
      throw new Error("storage");
    });
    const first = vi.fn();
    failing.fire({ id: 2 }, first);
    await new Promise((r) => setTimeout(r, 10));
    expect(first).toHaveBeenCalledTimes(1);

    vi.useFakeTimers();
    try {
      const slow = chromeApi();
      onNewDownload(slow, () => new Promise(() => {}));
      const second = vi.fn();
      slow.fire({ id: 3 }, second);
      await vi.advanceTimersByTimeAsync(DECIDE_MS - 1);
      expect(second).not.toHaveBeenCalled();
      await vi.advanceTimersByTimeAsync(1);
      expect(second).toHaveBeenCalledTimes(1);
    } finally {
      vi.useRealTimers();
    }
  });
});

describe("a YouTube playlist's page", () => {
  it("is that of a playlist's page, or of a video played from a playlist, on the same site", () => {
    expect(playlistPage("https://www.youtube.com/playlist?list=PLx_1-a")).toBe("https://www.youtube.com/playlist?list=PLx_1-a");
    expect(playlistPage("https://www.youtube.com/watch?v=abc&list=PLx&index=3&pp=iAQB")).toBe("https://www.youtube.com/playlist?list=PLx");
    expect(playlistPage("https://m.youtube.com/watch?list=OLAK5uy_x&v=abc")).toBe("https://m.youtube.com/playlist?list=OLAK5uy_x");
    expect(playlistPage("https://youtube.com/playlist?list=WL")).toBe("https://youtube.com/playlist?list=WL");
    // YouTube Music's lists have a page; only a Mix does not.
    expect(playlistPage("https://www.youtube.com/watch?v=abc&list=RDCLAK5uy_abc")).toBe("https://www.youtube.com/playlist?list=RDCLAK5uy_abc");
  });

  it("is none for a Mix, another page or site, or a list id YouTube never makes", () => {
    for (const url of [
      "https://www.youtube.com/watch?v=abc&list=RDabc&start_radio=1",
      "https://www.youtube.com/watch?v=abc",
      "https://www.youtube.com/watch?v=abc&list=TLPQMjgxMDIwMjY",
      "https://www.youtube.com/watch?v=abc&list=RDMMabc",
      "https://www.youtube.com/playlist",
      "https://www.youtube.com/@channel/playlists?list=PLx",
      "https://www.youtube.com/embed/videoseries?list=PLx",
      "http://www.youtube.com/playlist?list=PLx",
      "https://youtube.com.example.test/playlist?list=PLx",
      "https://music.youtube.com/playlist?list=PLx",
      "https://www.youtube.com/playlist?list=PL%2F..%2Fx",
      "https://www.youtube.com/playlist?list=P",
      `https://www.youtube.com/playlist?list=${"a".repeat(201)}`,
      "https://user:pass@www.youtube.com/playlist?list=PLx",
      undefined,
    ]) {
      expect(playlistPage(url), String(url)).toBeNull();
    }
  });
});
