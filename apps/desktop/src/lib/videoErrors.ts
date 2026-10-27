// What «Video quality» says of a failure: a sentence in the user's
// language for what yt-dlp's words come to, and its own text only under
// «Details».
import type { VideoErrorView } from "./bindings/VideoErrorView";

const SENTENCE: Record<string, string> = {
  drm: "video-drm",
  tools_missing: "video-tools-missing",
  no_formats: "video-error-no_formats",
  sign_in: "video-error-sign_in",
  private: "video-error-private",
  unsupported: "video-error-unsupported",
  challenge: "video-error-challenge",
};

/** The sentence's key: its own for a known failure, else a general one. */
export function errorKey(e: VideoErrorView | string): string {
  return (typeof e === "string" ? undefined : SENTENCE[e.kind]) ?? "video-error-other";
}

/** The words under «Details»: yt-dlp's (or Zajil's) own text, but not
 * for what Zajil itself says whole (DRM, the tools). */
export function errorDetails(e: VideoErrorView | string): string | null {
  if (typeof e === "string") return e.trim() || null;
  if (e.kind === "drm" || e.kind === "tools_missing") return null;
  return e.message.trim() || null;
}
