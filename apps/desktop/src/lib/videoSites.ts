// A link to a video's page (owner's report, 2026-10-30: a YouTube link
// added by hand gave «this link is a web page»): it goes to «Video quality»,
// as the browser's button sends it, not to «Download file info». Only the
// big sites' video pages, by their own paths; any other page still offers
// «Download the video» once it turns out to be a page.
import pages from "./videoPages.json";

// The list is `videoPages.json`, which Rust reads too («Batch download»
// refuses a pattern whose links are these pages, #332): each entry is the
// site's host, then its videos' paths, as regular expressions.
const PAGES: [RegExp, RegExp][] = (pages as [string, string][]).map(([host, path]) => [new RegExp(host), new RegExp(path)]);

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
