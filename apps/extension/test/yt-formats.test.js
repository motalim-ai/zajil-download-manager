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

  // The owner's video of 2026-10-29: one language, no track marked the
  // default; its pictures were left out for want of a sound.
  it("takes a sound with a language as the video's own when no track is the default", () => {
    const track = { name: "English (US)", main: false };
    const list = load()({
      ...page,
      formats: page.formats.map((f) => (f.mime.startsWith("audio/") ? { ...f, track } : f)),
    });
    expect(list.video.map((/** @type {any} */ c) => c.send)).toContainEqual({ video: "299", audio: "140" });
    expect(list.audio.map((/** @type {any} */ c) => c.name)).toEqual(["WEBM 140 kbps", "M4A 130 kbps"]);
  });

  // A video with its own sound and an AI dub, in the same itags (owner's
  // request, 2026-10-31): the list offers the sound's language, the
  // video's own first, and a dub goes with its language to Zajil.
  it("offers a dub's language, and sends it with the dub's sound", () => {
    const own = { name: "English (US) original", main: true, lang: "en-US.4" };
    const dub = { name: "العربية", main: false, lang: "ar.3" };
    const sounds = page.formats.filter((f) => f.mime.startsWith("audio/") && !f.drc);
    const data = {
      ...page,
      formats: [
        ...page.formats.filter((f) => !f.mime.startsWith("audio/")),
        ...sounds.map((f) => ({ ...f, track: dub, size: String(9 * MB) })),
        ...sounds.map((f) => ({ ...f, track: own })),
      ],
    };
    const make = load();
    const list = make(data);
    expect(list.tracks).toEqual([
      { name: "English (US) original", lang: "en-US", main: true },
      { name: "العربية", lang: "ar", main: false },
    ]);
    expect(list.lang).toBe("");
    expect(list.video[0].send).toEqual({ video: "299", audio: "140" });
    expect(list.audio.map((/** @type {any} */ c) => c.send)).toEqual([{ id: "251" }, { id: "140" }]);
    const dubbed = make(data, "ar");
    expect(dubbed.lang).toBe("ar");
    // 360p has the video's own sound inside: left out with a dub.
    expect(dubbed.video.map((/** @type {any} */ c) => c.send)).toEqual([
      { video: "299", audio: "140", language: "ar" },
      { video: "303", audio: "251", language: "ar" },
    ]);
    expect(dubbed.audio.map((/** @type {any} */ c) => c.send)).toEqual([
      { id: "251", language: "ar" },
      { id: "140", language: "ar" },
    ]);
    // A language the page lacks, or one language only: the video's own.
    expect(make(data, "fr").video[0].send).toEqual({ video: "299", audio: "140" });
    expect(make(page).tracks).toEqual([]);
    // The original chosen by its name goes with its language too.
    expect(make(data, "en-US").video[0].send).toEqual({ video: "299", audio: "140", language: "en-US" });
  });

  // YouTube marks the dub in the viewer's language as the default (owner's
  // report, 2026-10-31: an Arabic dub chosen came as English): a language
  // chosen goes with its name to Zajil all the same.
  it("sends a chosen dub's language even when YouTube marks that dub the default", () => {
    const own = { name: "English (United States) original", main: false, lang: "en-US.4" };
    const dub = { name: "Arabic", main: true, lang: "ar.3" };
    const sounds = page.formats.filter((f) => f.mime.startsWith("audio/") && !f.drc);
    const data = {
      ...page,
      formats: [...page.formats.filter((f) => !f.mime.startsWith("audio/")), ...sounds.map((f) => ({ ...f, track: dub })), ...sounds.map((f) => ({ ...f, track: own }))],
    };
    const make = load();
    expect(make(data).video[0].send).toEqual({ video: "299", audio: "140" });
    expect(make(data, "ar").video[0].send).toEqual({ video: "299", audio: "140", language: "ar" });
    expect(make(data, "ar").audio.map((/** @type {any} */ c) => c.send.language)).toEqual(["ar", "ar"]);
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
