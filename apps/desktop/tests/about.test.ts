import { describe, expect, it } from "vitest";
import { licenseOf, toolLines } from "../src/lib/about";
import type { ToolView } from "../src/lib/bindings/ToolView";

// «About»'s video tools (#168): the keys and values are what is checked;
// the texts themselves are the translations test's.
const say = (key: string, args?: Record<string, string>) => (args ? `${key}(${Object.entries(args).map(([k, v]) => `${k}=${v}`).join(", ")})` : key);
const tool = (tool: string, version: string | null, source: string): ToolView => ({ tool, installed: false, version, host: "github.com", source });

describe("About's video tools", () => {
  const tools = [
    tool("yt-dlp", "2026.10.10", "https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.10"),
    tool("ffmpeg", "8.1.3-1", "https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1"),
    tool("ffprobe", "8.1.3-1", "https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1"),
    tool("qjs", "v0.10.1", "https://github.com/quickjs-ng/quickjs/releases/tag/v0.10.1"),
  ];

  it("names each tool once, with its version, license and source; ffprobe is FFmpeg's", () => {
    const lines = toolLines(say, tools);
    expect(lines.filter((l) => l.startsWith("about-tool("))).toEqual([
      "about-tool(tool=yt-dlp, version=2026.10.10, license=tool-license-yt-dlp(binary=GPLv3+, source=Unlicense), source=https://github.com/yt-dlp/yt-dlp/releases/tag/2026.10.10)",
      "about-tool(tool=FFmpeg, version=8.1.3-1, license=about-or-later(license=LGPL 2.1), source=https://github.com/motalim-ai/zajil-download-manager/releases/tag/tools-ffmpeg-8.1.3-1)",
      "about-tool(tool=QuickJS-NG, version=v0.10.1, license=MIT, source=https://github.com/quickjs-ng/quickjs/releases/tag/v0.10.1)",
    ]);
  });

  it("says FFmpeg's sentence, and that the tools are their owners'", () => {
    const lines = toolLines(say, tools);
    expect(lines).toContain("about-ffmpeg(license=LGPLv2.1)");
    const owners = lines.slice(lines.indexOf("about-not-ours") + 1);
    expect(owners).toEqual(["yt-dlp: https://github.com/yt-dlp/yt-dlp", "FFmpeg: https://ffmpeg.org", "QuickJS-NG: https://github.com/quickjs-ng/quickjs"]);
  });

  it("adds nothing without tools", () => {
    expect(toolLines(say, [])).toEqual([]);
  });

  it("gives each tool one license text, the one the question before downloading and the settings show", () => {
    expect(licenseOf(say, "yt-dlp")).toBe("tool-license-yt-dlp(binary=GPLv3+, source=Unlicense)");
    expect(licenseOf(say, "ffmpeg")).toBe("about-or-later(license=LGPL 2.1)");
    expect(licenseOf(say, "ffprobe")).toBe(licenseOf(say, "ffmpeg"));
    expect(licenseOf(say, "qjs")).toBe("MIT");
    for (const t of tools.filter((t) => t.tool !== "ffprobe")) {
      expect(toolLines(say, [t])[1]).toContain(`license=${licenseOf(say, t.tool)}, source=`);
    }
  });
});
