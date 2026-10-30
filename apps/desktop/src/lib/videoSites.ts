// A link to a video's page (owner's report, 2026-10-30: a YouTube link
// added by hand gave «this link is a web page»): it goes to «Video quality»,
// as the browser's button sends it, not to «Download file info». Only the
// big sites' video pages, by their own paths; any other page still offers
// «Download the video» once it turns out to be a page.
const PAGES: [RegExp, RegExp][] = [
  [/^(www\.|m\.|music\.)?youtube\.com$/, /^\/(watch|shorts\/|live\/|embed\/|playlist)/],
  [/^youtu\.be$/, /^\/[\w-]+/],
  [/^(www\.|mobile\.)?(x|twitter)\.com$/, /^\/[^/]+\/status\/\d+/],
  [/^(www\.)?instagram\.com$/, /^\/(p|reel|reels|tv)\//],
  [/^(www\.|m\.|web\.)?facebook\.com$/, /^\/(.+\/videos\/|watch|reel\/|share\/(v|r)\/)/],
  [/^fb\.watch$/, /^\/[\w-]+/],
  [/^(www\.|m\.)?tiktok\.com$/, /^\/@[^/]+\/video\/\d+/],
  [/^(vm|vt)\.tiktok\.com$/, /^\/[\w-]+/],
  [/^(www\.)?vimeo\.com$/, /^\/\d+/],
  [/^(www\.)?dailymotion\.com$/, /^\/video\//],
];

export function isVideoPage(link: string): boolean {
  let url: URL;
  try {
    url = new URL(link.trim());
  } catch {
    return false;
  }
  if (url.protocol !== "https:" && url.protocol !== "http:") return false;
  const host = url.hostname.toLowerCase();
  return PAGES.some(([h, p]) => h.test(host) && p.test(url.pathname));
}
