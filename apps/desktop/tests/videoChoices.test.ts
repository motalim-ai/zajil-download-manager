import { describe, expect, it } from "vitest";
import { fileName, notForQuickTime, options, picked, tracks } from "../src/lib/videoChoices";
import type { FormatView } from "../src/lib/bindings/FormatView";

// «Video quality»'s choices (M6, #149).
const f = (id: string, kind: string, height: number | null, ext: string, size: number | null): FormatView => ({ id, ext, width: null, height, fps: 30, size, kind, hls: false, codec: null, original: false, language: null });

describe("the quality choices", () => {
  const formats = [
    f("18", "both", 360, "mp4", 10),
    f("137", "video", 1080, "mp4", 100),
    f("136", "video", 720, "mp4", 50),
    f("22", "both", 720, "mp4", null),
    f("251", "audio", null, "webm", 7),
    f("140", "audio", null, "m4a", 5),
  ];

  it("merges picture-only formats with the best sound, m4a first, and puts sound only last", () => {
    const list = options(formats);
    expect(list.map((o) => o.height)).toEqual([1080, 720, 360, 0]);
    expect(list[0]!.choice).toEqual({ kind: "merge", video: "137", audio: "140" });
    expect(list[0]!.size).toBe(105);
    expect(list.at(-1)!.choice).toEqual({ kind: "audio_only" });
  });

  it("merges the video's own sound, not a dubbed one, even a larger one (users' reports, 2026-10-31)", () => {
    const own = { ...f("140-1", "audio", null, "m4a", 5), original: true };
    const dubbed = f("140-0", "audio", null, "m4a", 6);
    const list = options([f("137", "video", 1080, "mp4", 100), dubbed, own]);
    expect(list[0]!.choice).toEqual({ kind: "merge", video: "137", audio: "140-1" });
    // No sound marked: the largest m4a, as before.
    expect(options([f("137", "video", 1080, "mp4", 100), dubbed, f("140-1", "audio", null, "m4a", 5)])[0]!.choice).toEqual({
      kind: "merge",
      video: "137",
      audio: "140-0",
    });
  });

  it("offers the dub one hears when chosen: merges with it, and it as sound only (owner's request, 2026-10-31)", () => {
    const sound = (id: string, ext: string, size: number, language: string, original: boolean): FormatView => ({ ...f(id, "audio", null, ext, size), language, original });
    const page = [
      f("18", "both", 360, "mp4", 10),
      f("137", "video", 1080, "mp4", 100),
      sound("140-1", "m4a", 5, "en", true),
      sound("251-1", "webm", 6, "en", true),
      sound("140-0", "m4a", 5, "ar", false),
    ];
    const found = tracks(page);
    expect(found).toEqual([
      { language: "en", original: true },
      { language: "ar", original: false },
    ]);
    const own = options(page, null, found[0]!);
    expect(own[0]!.choice).toEqual({ kind: "merge", video: "137", audio: "140-1" });
    expect(own.at(-1)!.choice).toEqual({ kind: "audio_only" });
    const dub = options(page, null, found[1]!);
    // 360p's own format holds the original sound: only merges with the dub.
    expect(dub.map((o) => o.height)).toEqual([1080, 0]);
    expect(dub[0]!.choice).toEqual({ kind: "merge", video: "137", audio: "140-0" });
    expect(dub.at(-1)!.choice).toEqual({ kind: "format", id: "140-0" });
    expect(dub.at(-1)!.sound).toBe(true);
    // One language only: nothing to choose.
    expect(tracks(page.filter((x) => x.language !== "ar"))).toEqual([]);
  });

  it("keeps each height once, the one with a size", () => {
    const at720 = options(formats).filter((o) => o.height === 720);
    expect(at720).toHaveLength(1);
    expect(at720[0]!.size).toBe(55);
  });

  it("picks the engine's best at first, else the first", () => {
    const list = options(formats);
    expect(picked(list, { kind: "format", id: "18" })).toBe(2);
    expect(picked(list, { kind: "format", id: "nope" })).toBe(0);
    expect(picked(list, null)).toBe(0);
  });

  it("always holds the engine's best and picks it, whatever comes first in the page", () => {
    // A YouTube page: AV1 listed before H.264 at 1080p (both .mp4), and a
    // small m4a before the one the engine chose.
    const page = [
      f("139", "audio", null, "m4a", 2),
      f("140", "audio", null, "m4a", 5),
      f("251", "audio", null, "webm", 7),
      f("399", "video", 1080, "mp4", 80),
      f("137", "video", 1080, "mp4", 100),
      f("616", "video", 1440, "mp4", 300),
    ];
    const best = { kind: "merge" as const, video: "137", audio: "140" };
    const list = options(page, best);
    expect(list[picked(list, best)]!.choice).toEqual(best);
    expect(list.filter((o) => o.choice.kind === "merge").every((o) => o.choice.kind === "merge" && o.choice.audio === "140")).toBe(true);
    // With no best from the engine, the largest m4a.
    expect(options(page)[0]!.choice).toEqual({ kind: "merge", video: "616", audio: "140" });
  });

  it("names the file after the title, with no extension (the engine adds yt-dlp's)", () => {
    expect(fileName("درس: الجزء 1/2")).toBe("درس الجزء 1 2");
    expect(fileName("Talk")).toBe("Talk");
    expect(fileName("")).toBe("video");
  });

  it("says which pictures QuickTime cannot play", () => {
    const o = (codec: string | null) => ({ choice: { kind: "best" as const }, height: 1080, ext: "mp4", codec, fps: null, size: null });
    expect(["VP9", "AV1", "H.264", null].map((c) => notForQuickTime(o(c)))).toEqual([true, true, false, false]);
  });
});
