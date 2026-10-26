// «About»'s video tools (#168, ADR-0009): each with its version, license
// and source, FFmpeg's sentence, and that the tools are their owners'.
import type { ToolView } from "./bindings/ToolView";

type Say = (key: string, args?: Record<string, string>) => string;

/** How each tool is named in «About». */
const NAMES: Record<string, string> = { "yt-dlp": "yt-dlp", ffmpeg: "FFmpeg", qjs: "QuickJS-NG" };

/** A tool's license, the one text for «About», the settings and the question
 * before downloading (ADR-0009). yt-dlp's file is its PyInstaller build,
 * which holds GPLv3+ code; its source code is the Unlicense. */
export function licenseOf(say: Say, tool: string): string {
  switch (tool) {
    case "yt-dlp":
      return say("tool-license-yt-dlp", { binary: "GPLv3+", source: "Unlicense" });
    case "ffmpeg":
    case "ffprobe":
      return say("about-or-later", { license: "LGPL 2.1" });
    case "qjs":
      return "MIT";
    default:
      return "—";
  }
}

/** The projects that own the tools. */
export const OWNERS: [string, string][] = [
  ["yt-dlp", "https://github.com/yt-dlp/yt-dlp"],
  ["FFmpeg", "https://ffmpeg.org"],
  ["QuickJS-NG", "https://github.com/quickjs-ng/quickjs"],
];

/** The lines «About» adds for the video tools (none if Zajil has none). */
export function toolLines(say: Say, tools: ToolView[]): string[] {
  const lines = tools.flatMap((t) => {
    const name = NAMES[t.tool];
    if (!name) return [];
    return [say("about-tool", { tool: name, version: t.version ?? "—", license: licenseOf(say, t.tool), source: t.source })];
  });
  if (lines.length === 0) return [];
  return [
    say("about-video-tools"),
    ...lines,
    "",
    say("about-ffmpeg", { license: "LGPLv2.1" }),
    "",
    say("about-not-ours"),
    ...OWNERS.map(([name, url]) => `${name}: ${url}`),
  ];
}
