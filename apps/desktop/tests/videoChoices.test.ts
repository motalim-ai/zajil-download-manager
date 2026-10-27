import { describe, expect, it } from "vitest";
import { fileName, options, picked } from "../src/lib/videoChoices";
import type { FormatView } from "../src/lib/bindings/FormatView";

// «Video quality»'s choices (M6, #149).
const f = (id: string, kind: string, height: number | null, ext: string, size: number | null): FormatView => ({ id, ext, width: null, height, fps: 30, size, kind, hls: false });

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
});
