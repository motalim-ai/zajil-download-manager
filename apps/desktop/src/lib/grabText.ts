// «Site grabber» (M7): what the user types made into lists, and why a
// check was refused or what limit it reached, in the user's words.
type Say = (key: string, args?: Record<string, string | number>) => string;

/** Items separated by commas or spaces, without the empty ones. */
export function listOf(text: string): string[] {
  return text
    .split(/[,،\s]+/)
    .map((t) => t.trim())
    .filter((t) => t !== "");
}

/** Why Rust refused a check (`start`, `depth`, `max_files`, `max_total`,
 * `types`, `site:NAME`), or its own text for anything else. */
export function refusalText(say: Say, error: string, limits: { maxDepth: number; maxFiles: number }): string {
  if (error.startsWith("site:")) return say("grab-error-site", { site: error.slice(5), example: "cdn.example.com" });
  switch (error) {
    case "start":
      return say("grab-error-start", { http: "http://", https: "https://" });
    case "depth":
      return say("grab-error-depth", { min: 1, max: limits.maxDepth });
    case "max_files":
      return say("grab-error-max_files", { min: 1, max: limits.maxFiles });
    case "max_total":
      return say("grab-error-max_total");
    case "types":
      return say("grab-error-types");
    default:
      return error;
  }
}

export const LIMITS = ["files", "pages", "total", "waiting", "seen"] as const;

/** What a reached limit means. */
export function limitText(say: Say, limit: string): string {
  return (LIMITS as readonly string[]).includes(limit) ? say(`grab-limit-${limit}`) : limit;
}
