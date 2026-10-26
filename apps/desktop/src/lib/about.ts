// The video tools' names, versions and licenses (#168, ADR-0009): one text
// for «About», the settings and the question before downloading; and
// «About»'s lines: each tool with its source, FFmpeg's sentence, and that
// the tools are their owners'.
import type { ToolView } from "./bindings/ToolView";

type Say = (key: string, args?: Record<string, string>) => string;

/** Left-to-right inside any text (LRI … PDI): «GPLv3+» keeps its «+» on
 * its right in Arabic, in the page and in the system's «About» box. */
export const ltr = (text: string): string => `⁦${text}⁩`;

/** Each tool's name as its project writes it (FFmpeg's legal checklist,
 * item 15), with its file's name. */
const NAMES: Record<string, string> = {
  "yt-dlp": "yt-dlp",
  ffmpeg: "FFmpeg (ffmpeg)",
  ffprobe: "FFprobe (ffprobe)",
  qjs: "QuickJS-NG (qjs)",
};

export function toolName(tool: string): string {
  return ltr(NAMES[tool] ?? tool);
}

/** The pinned version, as its project numbers it (QuickJS-NG's tag is
 * `v0.17.0`, its version 0.17.0). */
export function versionOf(t: ToolView): string {
  return t.version ? ltr(t.version.replace(/^v(?=\d)/, "")) : "—";
}

/** A tool's license, for the two tables; «About» says the same licenses
 * in its own lines (`aboutLine`, checked by a test). yt-dlp's file is its PyInstaller build, which holds
 * GPLv3+ code; its source code is the Unlicense. */
export function licenseOf(say: Say, tool: string): string {
  switch (tool) {
    case "yt-dlp":
      return say("tool-license-yt-dlp", { binary: "GPLv3+", source: "Unlicense" });
    case "ffmpeg":
    case "ffprobe":
      return say("about-or-later", { license: "LGPL 2.1" });
    case "qjs":
      return ltr("MIT");
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

/** «About»'s line for one tool. Each name is its own value, so each is
 * isolated (Fluent's FSI … PDI); a license sentence is never put inside
 * another text, where its Arabic would sit in a left-to-right isolate. */
function aboutLine(say: Say, t: ToolView): string {
  const args = { tool: NAMES[t.tool] ?? t.tool, version: t.version?.replace(/^v(?=\d)/, "") ?? "—", source: t.source };
  switch (t.tool) {
    case "yt-dlp":
      return say("about-tool-yt-dlp", { ...args, binary: "GPLv3+", license: "Unlicense" });
    case "ffmpeg":
    case "ffprobe":
      return say("about-tool-or-later", { ...args, license: "LGPL 2.1" });
    default:
      return say("about-tool", { ...args, license: "MIT" });
  }
}

/** The lines «About» adds for the video tools (none if Zajil has none). */
export function toolLines(say: Say, tools: ToolView[]): string[] {
  const lines = tools.filter((t) => t.tool in NAMES).map((t) => aboutLine(say, t));
  if (lines.length === 0) return [];
  return [
    say("about-video-tools"),
    ...lines,
    "",
    say("about-ffmpeg", { license: "LGPLv2.1" }),
    "",
    say("about-not-ours"),
    ...OWNERS.map(([name, url]) => ltr(`${name}: ${url}`)),
  ];
}
