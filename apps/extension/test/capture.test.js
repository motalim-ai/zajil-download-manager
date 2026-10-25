import { describe, expect, it, vi } from "vitest";
import {
  HOST,
  MAX_URL,
  NO_ANSWER,
  downloadMessage,
  extensionOf,
  send,
  shouldCapture,
  takeOver,
} from "../src/capture.js";
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
    ]) {
      expect(shouldCapture(item(over), s, "zajil-self"), JSON.stringify(over)).toBe(false);
    }
    expect(shouldCapture(item(), { ...s, capture: false })).toBe(false);
    expect(shouldCapture(item(), { ...s, extensions: ["zip"] })).toBe(false);
  });

  it("respects excluded sites, for the file and for the page it came from", () => {
    const s = { ...defaults(), excludedSites: ["example.com"] };
    expect(shouldCapture(item(), s)).toBe(false);
    expect(shouldCapture(item({ referrer: "https://other.test/", finalUrl: "https://dl.example.com/a.zip" }), s)).toBe(false);
    expect(shouldCapture(item({ referrer: "https://other.test/", finalUrl: "https://badexample.com/a.zip", url: "https://badexample.com/a.zip" }), s)).toBe(true);
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
      { filename: ".." },
      { filename: `${"ز".repeat(128)}.zip` },
    ]) {
      expect(Object.keys(downloadMessage("https://example.com/a.zip", extra)).sort(), JSON.stringify(extra)).toEqual(["protocol_version", "type", "url"]);
    }
  });
});

describe("handing a download over", () => {
  it("taken by Zajil: the browser's download is paused, then cancelled and removed", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: true } });
    expect(await takeOver(api, item())).toEqual({ outcome: "taken" });
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "cancel", "erase"]);
    expect(api.calls[1][1]).toBe(HOST);
    expect(api.calls[1][2].url).toBe("https://cdn.example.com/files/setup.exe");
  });

  it("refused by Zajil: the browser goes on with it", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: false, error: "the URL has control characters" } });
    expect((await takeOver(api, item())).outcome).toBe("refused");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "resume"]);
  });

  it("no host (Zajil not installed): the browser goes on with it", async () => {
    const api = fakeApi({ nativeError: "Specified native messaging host not found." });
    expect((await takeOver(api, item())).outcome).toBe("unreachable");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native", "resume"]);
  });

  it("no answer in time: it stays paused, since Zajil may have it (#64)", async () => {
    const api = fakeApi({ answer: { protocol_version: 1, ok: false, error: `${NO_ANSWER} 10 s` } });
    expect((await takeOver(api, item())).outcome).toBe("unknown");
    expect(api.calls.map((c) => c[0])).toEqual(["pause", "native"]);
  });

  it("already finished in the browser: nothing is sent", async () => {
    const api = fakeApi({ pauseError: "Download must be in progress" });
    expect(await takeOver(api, item())).toBeNull();
    expect(api.calls.map((c) => c[0])).toEqual(["pause"]);
  });

  it("a malformed answer counts as refused", async () => {
    expect((await send(fakeApi({ answer: undefined }), {})).outcome).toBe("refused");
    expect((await send(fakeApi({ answer: { ok: "yes" } }), {})).outcome).toBe("refused");
  });
});
