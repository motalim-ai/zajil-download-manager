import { describe, expect, it } from "vitest";
import { isVideoPage } from "../src/lib/videoSites";

// A video's page goes to «Video quality» (owner's report, 2026-10-30).
describe("a video's page", () => {
  it("is known on the big sites, by their video paths", () => {
    for (const link of [
      "https://www.youtube.com/watch?v=x1Yjj1D27Xo&t=942s",
      "https://youtu.be/x1Yjj1D27Xo",
      "https://www.youtube.com/shorts/abc",
      "https://x.com/someone/status/123",
      "https://twitter.com/someone/status/123",
      "https://www.instagram.com/reel/DfF60hBI4ge/",
      "https://www.facebook.com/100003352791153/videos/1234/",
      "https://fb.watch/abc/",
      "https://www.tiktok.com/@a/video/123",
      "https://vimeo.com/76979871",
    ]) {
      expect(isVideoPage(link), link).toBe(true);
    }
  });

  it("is not a file, another page of those sites, or anything else", () => {
    for (const link of [
      "https://github.com/motalim-ai/zajil-download-manager/releases/download/v1.1.3/Zajil_1.1.3_universal.dmg",
      "https://www.youtube.com/",
      "https://x.com/motalim_ai",
      "https://www.instagram.com/someone/",
      "ftp://youtube.com/watch",
      "not a link",
    ]) {
      expect(isVideoPage(link), link).toBe(false);
    }
  });
});
