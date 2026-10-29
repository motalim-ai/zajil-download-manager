// The video button's list (`yt-formats.js`): what YouTube's player says of
// its formats, checked and made into choices with their sizes.
import { readFileSync } from "node:fs";
import { runInNewContext } from "node:vm";
import { describe, expect, it } from "vitest";

const script = readFileSync(new URL("../src/yt-formats.js", import.meta.url), "utf8");
/** @returns {(data: any) => any} */
const load = () => {
  /** @type {any} */
  const world = {};
  world.globalThis = world;
  runInNewContext(script, world);
  return world.__zajilFormats.choicesOf;
};

const MB = 1024 * 1024;
const page = {
  video: "aqz-KE-bpKQ",
  title: "Big Buck Bunny",
  seconds: "634",
  formats: [
    { itag: 18, mime: 'video/mp4; codecs="avc1.42001E, mp4a.40.2"', bitrate: 500000, size: String(27 * MB), height: 360, quality: "360p" },
    { itag: 299, mime: 'video/mp4; codecs="avc1.64002a"', bitrate: 3000000, size: String(245 * MB), height: 1080, fps: 60, quality: "1080p60" },
    { itag: 303, mime: 'video/webm; codecs="vp9"', bitrate: 2000000, size: String(160 * MB), height: 1080, fps: 60, quality: "1080p60" },
    { itag: 140, mime: 'audio/mp4; codecs="mp4a.40.2"', bitrate: 130000, size: String(10 * MB) },
    { itag: 251, mime: 'audio/webm; codecs="opus"', bitrate: 140000, size: String(11 * MB) },
    { itag: 140, mime: 'audio/mp4; codecs="mp4a.40.2"', bitrate: 130000, size: String(10 * MB), drc: true },
    { itag: 999, mime: 'video/mp4; codecs="avc1"', drm: true },
  ],
};

describe("the video button's list", () => {
  it("pairs each picture with its sound, with the size of both, best first", () => {
    const list = load()(page);
    expect(list.title).toBe("Big Buck Bunny");
    expect(list.duration).toBe("10:34");
    expect(list.video.map((/** @type {any} */ c) => [c.name, c.size, c.send])).toEqual([
      ["MP4 1080p60", "255 MB", { video: "299", audio: "140" }],
      ["WEBM 1080p60", "171 MB", { video: "303", audio: "251" }],
      ["MP4 360p", "27.0 MB", { id: "18" }],
    ]);
    expect(list.video[0].detail).toBe("H.264 · 3.1 Mbps");
    // The "stable volume" sound and a protected format are left out.
    expect(list.audio.map((/** @type {any} */ c) => c.send.id)).toEqual(["251", "140"]);
  });

  it("checks what the page says: nothing for a live stream, no format, or nonsense", () => {
    const choicesOf = load();
    expect(choicesOf(null)).toBeNull();
    expect(choicesOf({ ...page, live: true })).toBeNull();
    expect(choicesOf({ formats: [{ itag: "x", mime: "video/mp4" }, { itag: 1, mime: 'text/html; codecs="a"' }] })).toBeNull();
    const odd = choicesOf({ title: "a‮b", formats: [{ itag: 140, mime: 'audio/mp4; codecs="mp4a.40.2"', size: "1e99", bitrate: -5 }] });
    expect(odd.title).toBe("ab");
    expect(odd.audio[0]).toMatchObject({ size: "", send: { id: "140" } });
  });
});
