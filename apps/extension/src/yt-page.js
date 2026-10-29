// @ts-check
// YouTube's own word on its video's formats, for the video button's list
// (owner's decision, 2026-10-29: the qualities show at once, with their
// sizes, without waiting for Zajil to read the page).
//
// This script runs in the page's own world (`world: "MAIN"`), on YouTube
// only, while "Allow Zajil on sites" lasts: only there is the player's
// answer to see (`getPlayerResponse`, else `ytInitialPlayerResponse`). It
// reads the formats' description (itag, type, size, rate, picture) and
// nothing else: no link, no cookie. Asked by `zajil-yt-ask` on the
// document, it answers `zajil-yt-formats` with that, as JSON text.
//
// What it says comes from the page, which may say anything: the button
// only shows it, every value checked there, and a press sends Zajil only
// a format's itag, which Zajil looks for in what it read itself.

(() => {
  const flag = "__zajilYtPage";
  if (/** @type {any} */ (window)[flag]) return;
  /** @type {any} */ (window)[flag] = true;

  /** The id of the video this page is about, from its address. */
  const pageVideo = () => {
    const { pathname, searchParams } = new URL(location.href);
    if (pathname === "/watch") return searchParams.get("v") ?? "";
    const m = /^\/(shorts|embed|live)\/([\w-]{6,20})/.exec(pathname);
    return m ? m[2] : "";
  };

  /** The player's answer for this page's video, if it has one. */
  const response = () => {
    const want = pageVideo();
    /** @type {any[]} */
    const found = [];
    for (const id of ["movie_player", "shorts-player"]) {
      try {
        const player = /** @type {any} */ (document.getElementById(id));
        if (typeof player?.getPlayerResponse === "function") found.push(player.getPlayerResponse());
      } catch {
        // not a player
      }
    }
    found.push(/** @type {any} */ (window).ytInitialPlayerResponse);
    return found.find((r) => r?.streamingData && (!want || r?.videoDetails?.videoId === want)) ?? null;
  };

  /** @param {any} f */
  const describe = (f) => ({
    itag: f.itag,
    mime: f.mimeType,
    bitrate: f.averageBitrate ?? f.bitrate,
    size: f.contentLength,
    width: f.width,
    height: f.height,
    fps: f.fps,
    quality: f.qualityLabel,
    audio: f.audioQuality,
    track: f.audioTrack ? { name: f.audioTrack.displayName, main: Boolean(f.audioTrack.audioIsDefault) } : undefined,
    drc: Boolean(f.isDrc),
    drm: Boolean(f.drmFamilies),
  });

  document.addEventListener("zajil-yt-ask", () => {
    const r = response();
    const data = r
      ? {
          video: r.videoDetails?.videoId,
          title: r.videoDetails?.title,
          seconds: r.videoDetails?.lengthSeconds,
          live: Boolean(r.videoDetails?.isLiveContent && r.videoDetails?.isLive),
          formats: [...(r.streamingData.formats ?? []), ...(r.streamingData.adaptiveFormats ?? [])].slice(0, 200).map(describe),
        }
      : null;
    document.dispatchEvent(new CustomEvent("zajil-yt-formats", { detail: JSON.stringify(data) }));
  });
})();
