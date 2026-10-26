import { describe, expect, it } from "vitest";
import { licenseOf, ltr, toolLines, toolName, versionOf } from "../src/lib/about";
import { format, LANGS } from "../src/lib/i18n.svelte";
import type { ToolView } from "../src/lib/bindings/ToolView";

// The video tools in «About», the settings and the question before
// downloading (#168), with the real texts of both languages.
const tool = (tool: string, version: string | null, source: string): ToolView => ({ tool, installed: false, version, host: "github.com", source });
const tools = [
  tool("yt-dlp", "2026.10.21", "https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.21"),
  tool("ffmpeg", "8.1.3-1", "https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1"),
  tool("ffprobe", "8.1.3-1", "https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1"),
  tool("qjs", "v0.17.0", "https://github.com/quickjs-ng/quickjs/releases/tag/v0.17.0"),
];
const sayIn = (lang: (typeof LANGS)[number]) => (key: string, args?: Record<string, string>) => format(lang, key, args);
// What sits outside every isolate (LRI or FSI … PDI), and what inside.
const OPEN = new Set(["\u2066", "\u2067", "\u2068"]);
const PDI = "\u2069";
function split(text: string): { outside: string; inside: string[] } {
  let depth = 0;
  let outside = "";
  const inside: string[] = [];
  for (const c of text) {
    if (OPEN.has(c)) {
      if (depth++ === 0) inside.push("");
    } else if (c === PDI) depth = Math.max(0, depth - 1);
    else if (depth === 0) outside += c;
    else inside[inside.length - 1] += c;
  }
  return { outside, inside };
}

describe("the video tools' texts", () => {
  it("name FFmpeg and FFprobe exactly, with their files' names", () => {
    expect(tools.map((t) => toolName(t.tool))).toEqual([ltr("yt-dlp"), ltr("FFmpeg (ffmpeg)"), ltr("FFprobe (ffprobe)"), ltr("QuickJS-NG (qjs)")]);
  });

  it("show the pinned versions as their projects number them", () => {
    expect(tools.map(versionOf)).toEqual([ltr("2026.10.21"), ltr("8.1.3-1"), ltr("8.1.3-1"), ltr("0.17.0")]);
    expect(versionOf(tool("qjs", null, ""))).toBe("—");
  });

  for (const lang of LANGS) {
    const say = sayIn(lang);

    it(`give each tool one license, in ${lang}`, () => {
      const yt = split(licenseOf(say, "yt-dlp")).inside;
      expect(yt).toEqual(["GPLv3+", "Unlicense"]);
      expect(split(licenseOf(say, "ffmpeg")).inside).toEqual(["LGPL 2.1"]);
      expect(licenseOf(say, "ffprobe")).toBe(licenseOf(say, "ffmpeg"));
      expect(licenseOf(say, "qjs")).toBe(ltr("MIT"));
    });

    it(`isolate every name, version, license and link, and nothing else, in ${lang}`, () => {
      // In Arabic, «GPLv3+» outside an isolate shows as «+GPLv3» (a plain
      // Latin word such as «FFmpeg» shows as it is); and Arabic
      // inside a left-to-right isolate shows its words in the wrong order.
      const all = [...toolLines(say, tools), ...tools.map((t) => licenseOf(say, t.tool)), ...tools.map((t) => toolName(t.tool) + versionOf(t))];
      for (const text of all) {
        const { outside, inside } = split(text);
        expect(outside, text).not.toMatch(/[0-9+/]/);
        for (const part of inside) expect(part, text).not.toMatch(/[\u0600-\u06FF]/);
      }
    });

    it(`say in «About» the same licenses as the tables, in ${lang}`, () => {
      const lines = toolLines(say, tools);
      const names = ["yt-dlp", "FFmpeg (ffmpeg)", "FFprobe (ffprobe)", "QuickJS-NG (qjs)"];
      tools.forEach((t, i) => {
        const line = lines.find((l) => split(l).inside[0] === names[i]);
        expect(line, t.tool).toBeDefined();
        const parts = split(line!).inside;
        expect(parts[1], t.tool).toBe(t.version!.replace(/^v/, ""));
        expect(parts.slice(2, -1), t.tool).toEqual(split(licenseOf(say, t.tool)).inside);
        expect(parts.at(-1), t.tool).toBe(t.source);
      });
      expect(lines.some((l) => split(l).inside.includes("LGPLv2.1"))).toBe(true);
      expect(lines.slice(-3)).toEqual([ltr("yt-dlp: https://github.com/yt-dlp/yt-dlp"), ltr("FFmpeg: https://ffmpeg.org"), ltr("QuickJS-NG: https://github.com/quickjs-ng/quickjs")]);
    });
  }

  it("adds nothing to «About» without tools", () => {
    expect(toolLines(sayIn("ar"), [])).toEqual([]);
  });
});
