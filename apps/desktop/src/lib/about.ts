// «About»'s video tools (#168, ADR-0009): each with its version, license
// and source, FFmpeg's sentence, and that the tools are their owners'.
import type { ToolView } from "./bindings/ToolView";

type Say = (key: string, args?: Record<string, string>) => string;

/** How each tool is named, and its license («or later» for FFmpeg's). */
const SHOWN: Record<string, { name: string; license: (say: Say) => string }> = {
  "yt-dlp": { name: "yt-dlp", license: () => "Unlicense" },
  ffmpeg: { name: "FFmpeg", license: (say) => say("about-or-later", { license: "LGPL 2.1" }) },
  qjs: { name: "QuickJS-NG", license: () => "MIT" },
};

/** The projects that own the tools. */
export const OWNERS: [string, string][] = [
  ["yt-dlp", "https://github.com/yt-dlp/yt-dlp"],
  ["FFmpeg", "https://ffmpeg.org"],
  ["QuickJS-NG", "https://github.com/quickjs-ng/quickjs"],
];

/** The lines «About» adds for the video tools (none if Zajil has none). */
export function toolLines(say: Say, tools: ToolView[]): string[] {
  const lines = tools.flatMap((t) => {
    const shown = SHOWN[t.tool];
    if (!shown) return [];
    return [say("about-tool", { tool: shown.name, version: t.version ?? "—", license: shown.license(say), source: t.source })];
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
