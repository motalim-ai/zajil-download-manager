import { describe, expect, it } from "vitest";
import { looksLikeAVideoPage } from "../src/lib/videoPage";

// What «Download file info» does with a web page's link now (the owner's
// test of a youtu.be link, which said «a web page»): the video sites' pages
// go to «Video quality», the rest stay a web page.
describe("looksLikeAVideoPage", () => {
  it("says yes for a YouTube video's page, in every form it has", () => {
    expect(looksLikeAVideoPage("https://youtu.be/2hFGsSB02JY")).toBe(true);
    expect(looksLikeAVideoPage("https://www.youtube.com/watch?v=2hFGsSB02JY")).toBe(true);
    expect(looksLikeAVideoPage("https://m.youtube.com/watch?v=2hFGsSB02JY")).toBe(true);
    expect(looksLikeAVideoPage("https://www.youtube.com/shorts/abc123")).toBe(true);
    expect(looksLikeAVideoPage("https://www.youtube.com/live/xyz456")).toBe(true);
    expect(looksLikeAVideoPage("https://www.youtube.com/playlist?list=PL1a2b3c")).toBe(true);
    expect(looksLikeAVideoPage("https://www.youtube-nocookie.com/embed/abc123")).toBe(true);
  });

  it("says no for YouTube's pages without a video", () => {
    expect(looksLikeAVideoPage("https://www.youtube.com/")).toBe(false);
    expect(looksLikeAVideoPage("https://www.youtube.com/feed/subscriptions")).toBe(false);
    expect(looksLikeAVideoPage("https://www.youtube.com/results?search_query=zajil")).toBe(false);
    expect(looksLikeAVideoPage("https://www.youtube.com/@motalim-ai")).toBe(false);
    expect(looksLikeAVideoPage("https://www.youtube.com/channel/UCabc")).toBe(false);
  });

  it("says yes for the other video sites' pages", () => {
    expect(looksLikeAVideoPage("https://vimeo.com/123456789")).toBe(true);
    expect(looksLikeAVideoPage("https://www.dailymotion.com/video/x8abcde")).toBe(true);
    expect(looksLikeAVideoPage("https://dai.ly/x8abcde")).toBe(true);
    expect(looksLikeAVideoPage("https://www.tiktok.com/@user/video/7300000000000000000")).toBe(true);
    expect(looksLikeAVideoPage("https://www.bilibili.com/video/BV1abc")).toBe(true);
    expect(looksLikeAVideoPage("https://rumble.com/v1abc-something.html")).toBe(true);
    expect(looksLikeAVideoPage("https://x.com/user/status/1800000000000000000")).toBe(true);
    expect(looksLikeAVideoPage("https://twitter.com/user/status/1700000000000000000")).toBe(true);
    expect(looksLikeAVideoPage("https://www.instagram.com/p/abc123def/")).toBe(true);
    expect(looksLikeAVideoPage("https://www.instagram.com/reel/abc123def/")).toBe(true);
    expect(looksLikeAVideoPage("https://www.facebook.com/watch/?v=123456789")).toBe(true);
    expect(looksLikeAVideoPage("https://www.facebook.com/Page/videos/987654/")).toBe(true);
    expect(looksLikeAVideoPage("https://fb.watch/abcxyz?si=token")).toBe(true);
    expect(looksLikeAVideoPage("https://vk.com/video-123456_789012345")).toBe(true);
    expect(looksLikeAVideoPage("https://www.twitch.tv/videos/987654321")).toBe(true);
    expect(looksLikeAVideoPage("https://www.twitch.tv/channel/clip/HappyCat")).toBe(true);
  });

  it("says no for the posts of the sites that post, and their other pages", () => {
    expect(looksLikeAVideoPage("https://x.com")).toBe(false);
    expect(looksLikeAVideoPage("https://x.com/user")).toBe(false);
    expect(looksLikeAVideoPage("https://twitter.com/user")).toBe(false);
    expect(looksLikeAVideoPage("https://www.instagram.com/")).toBe(false);
    expect(looksLikeAVideoPage("https://www.facebook.com/profile.php?id=10")).toBe(false);
    expect(looksLikeAVideoPage("https://www.twitch.tv/channel")).toBe(false);
    expect(looksLikeAVideoPage("https://vk.com/feed")).toBe(false);
  });

  it("does not let another site name a video site (its host names it)", () => {
    expect(looksLikeAVideoPage("https://notyoutube.com/watch?v=1")).toBe(false);
    expect(looksLikeAVideoPage("https://youtu.be.evil.com/watch")).toBe(false);
    expect(looksLikeAVideoPage("https://example.com/youtu.be/2hFGsSB02JY")).toBe(false);
    expect(looksLikeAVideoPage("https://youtube.com.example.com/watch?v=1")).toBe(false);
  });

  it("says no for what is not a web address", () => {
    expect(looksLikeAVideoPage("")).toBe(false);
    expect(looksLikeAVideoPage("not a link")).toBe(false);
    expect(looksLikeAVideoPage("ftp://youtu.be/2hFGsSB02JY")).toBe(false);
    expect(looksLikeAVideoPage("file:///C:/Downloads/watch.html")).toBe(false);
  });
});
