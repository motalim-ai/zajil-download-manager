// A link the server answered with a web page may still be a video page
// (the owner's test with a youtu.be link): «Video quality» (M6, ADR-0009)
// reads video sites the engine's plain probe cannot. This decides it
// without asking yt-dlp, so it costs nothing: a known video page goes to
// «Video quality» at once, and every other page keeps its usual warning
// («add-web-page» in the add dialog) with «Download a video…» for it.

/** The sites whose pages are video pages: the host (its subdomains), and
 * on those where a page without a video is the rule (YouTube above all:
 * its previews play as the pointer passes), only the pages of videos and
 * of playlists — as the extension's `wantedHere` and `playlistPage` in
 * its video-button.js and capture.js say. */
const SITES: { site: RegExp; only?: RegExp }[] = [
  // A video's own page, a Short, a going live, an embedded player, or a
  // playlist's page.
  { site: /(^|\.)(youtube\.com|youtube-nocookie\.com)$/, only: /^\/(?:watch\b|shorts\/|live\/|embed\/|playlist\b)/ },
  // Links of its length (`youtu.be/<id>`): every path but no one is an id.
  { site: /^youtu\.be$/, only: /^\/[^/]/ },
  { site: /(^|\.)vimeo\.com$/ },
  { site: /(^|\.)dailymotion\.com$/ },
  { site: /^dai\.ly$/ },
  { site: /(^|\.)tiktok\.com$/ },
  { site: /(^|\.)bilibili\.com$/ },
  { site: /(^|\.)rumble\.com$/ },
  // A post that plays, not the site: x.com/<account>/status/<id>
  { site: /(^|\.)(x\.com|twitter\.com)$/, only: /\/status\/[^/]+/ },
  { site: /(^|\.)instagram\.com$/, only: /^\/[\w.-]*\/?(?:p|reel|reels|tv)\/[^/]+/ },
  {
    site: /(^|\.)facebook\.com$/,
    only: /^\/(?:[^/]+\/)?(?:watch\b|video\.php|reel\b|videos\/|share\/v\/)/,
  },
  { site: /^fb\.watch$/ },
  { site: /(^|\.)vk\.com$/, only: /^\/video/ },
  { site: /(^|\.)twitch\.tv$/, only: /^\/(?:[^/]+\/)?(?:v|videos|clip)\// },
];

/** Whether `url`'s page is one the sites above read videos from: then
 * «Download file info» hands it to «Video quality» at once (the owner's
 * test: a youtu.be link came out «a web page» in the add dialog). */
export function looksLikeAVideoPage(url: string): boolean {
  let page: URL;
  try {
    page = new URL(url);
  } catch {
    return false;
  }
  if (page.protocol !== "https:" && page.protocol !== "http:") return false;
  for (const { site, only } of SITES) {
    if (!site.test(page.host)) continue;
    return only ? only.test(page.pathname) : true;
  }
  return false;
}
