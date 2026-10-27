import { describe, expect, it } from "vitest";
import { errorDetails, errorKey } from "../src/lib/videoErrors";

describe("a video's failure", () => {
  it("is a sentence of its own when yt-dlp's words say what it is", () => {
    const e = { kind: "no_formats", message: "yt-dlp: [youtube] abc: Requested format is not available" };
    expect(errorKey(e)).toBe("video-error-no_formats");
    expect(errorKey({ kind: "sign_in", message: "x" })).toBe("video-error-sign_in");
    expect(errorKey({ kind: "private", message: "x" })).toBe("video-error-private");
    expect(errorKey({ kind: "unsupported", message: "x" })).toBe("video-error-unsupported");
    expect(errorKey({ kind: "challenge", message: "x" })).toBe("video-error-challenge");
    expect(errorDetails(e)).toBe(e.message);
  });

  it("is a general sentence otherwise, with yt-dlp's text under «Details»", () => {
    const e = { kind: "tool", message: "yt-dlp: HTTP Error 500" };
    expect(errorKey(e)).toBe("video-error-other");
    expect(errorDetails(e)).toBe("yt-dlp: HTTP Error 500");
    expect(errorKey("network down")).toBe("video-error-other");
    expect(errorDetails("network down")).toBe("network down");
  });

  it("needs no details where Zajil says it all", () => {
    expect(errorKey({ kind: "drm", message: "protected" })).toBe("video-drm");
    expect(errorDetails({ kind: "drm", message: "protected" })).toBeNull();
    expect(errorDetails({ kind: "tools_missing", message: "x" })).toBeNull();
  });
});
