// @ts-check
// The video button's list (owner's decision, 2026-10-29): what YouTube's
// player says of its formats (`yt-page.js`) made into the choices the
// list shows, each with its size and rate. Every value comes from the page
// and is checked here: a number must be one, and in reason; a text is cut
// short. Loaded before `video-button.js`, in the same content-script world
// (the page cannot reach it).

(() => {
  const g = /** @type {any} */ (globalThis);
  if (g.__zajilFormats) return;

  /** @param {unknown} v @param {number} max */
  const num = (v, max) => {
    const n = typeof v === "string" && /^\d{1,15}$/.test(v) ? Number(v) : v;
    return typeof n === "number" && Number.isFinite(n) && n >= 0 && n <= max ? n : undefined;
  };
  /** @param {unknown} v @param {number} max */
  const text = (v, max) => (typeof v === "string" ? v.replace(/[\u0000-\u001f\u007f‪-‮⁦-⁩]/g, "").slice(0, max) : "");

  /** What a codec is called. @param {string} codecs */
  const codecName = (codecs) => {
    if (/^avc1/.test(codecs)) return "H.264";
    if (/^(vp9|vp09)/.test(codecs)) return "VP9";
    if (/^av01/.test(codecs)) return "AV1";
    if (/^mp4a/.test(codecs)) return "AAC";
    if (/^opus/.test(codecs)) return "Opus";
    return codecs.split(".")[0].toUpperCase();
  };

  /** One format as the page described it, checked; or null.
   * @param {any} f */
  const checked = (f) => {
    const itag = num(f?.itag, 9999);
    const mime = /^(video|audio)\/(mp4|webm|3gpp); codecs="([\w.,\s-]{1,80})"$/.exec(text(f?.mime, 120));
    if (itag === undefined || !mime || f?.drm === true) return null;
    const codecs = mime[3].split(",").map((c) => c.trim());
    const isVideo = mime[1] === "video";
    return {
      itag: String(itag),
      ext: mime[1] === "audio" && mime[2] === "mp4" ? "m4a" : mime[2],
      kind: isVideo ? (codecs.length > 1 ? "both" : "video") : "audio",
      codec: codecName(codecs[0]),
      // Sound a player plays everywhere: AAC (not HE or 5.1) or Opus.
      plain: /^(mp4a\.40\.2|opus)$/.test(codecs[0]),
      bitrate: num(f.bitrate, 1e9),
      size: num(f.size, 1e12),
      height: num(f.height, 10000),
      fps: num(f.fps, 1000),
      quality: text(f.quality, 24),
      // A sound's track: its name, whether it is the video's own, and its
      // language as yt-dlp names it (`ar` of YouTube's `ar.3`).
      track:
        f.track && typeof f.track === "object"
          ? { name: text(f.track.name, 60), main: f.track.main === true, lang: /^([a-z]{2,3}(?:-[A-Za-z0-9]{1,8})?)(?:\.|$)/i.exec(text(f.track.lang, 40))?.[1] ?? "" }
          : null,
      drc: f.drc === true,
    };
  };

  /** @param {number | undefined} bytes */
  const sizeText = (bytes) => {
    if (bytes === undefined) return "";
    if (bytes >= 1024 ** 3) return `${(bytes / 1024 ** 3).toFixed(2)} GB`;
    if (bytes >= 1024 ** 2) return `${(bytes / 1024 ** 2).toFixed(bytes >= 100 * 1024 ** 2 ? 0 : 1)} MB`;
    return `${Math.max(1, Math.round(bytes / 1024))} KB`;
  };
  /** @param {number | undefined} bits */
  const rateText = (bits) => {
    if (bits === undefined) return "";
    return bits >= 1e6 ? `${(bits / 1e6).toFixed(1)} Mbps` : `${Math.round(bits / 1000)} kbps`;
  };
  /** @param {number | undefined} seconds */
  const durationText = (seconds) => {
    if (seconds === undefined) return "";
    const s = Math.floor(seconds % 60);
    const m = Math.floor(seconds / 60) % 60;
    const h = Math.floor(seconds / 3600);
    const two = (/** @type {number} */ n) => String(n).padStart(2, "0");
    return h ? `${h}:${two(m)}:${two(s)}` : `${m}:${two(s)}`;
  };

  const CODEC_ORDER = ["H.264", "VP9", "AV1"];

  /**
   * The list's choices for what `yt-page.js` said, or null when it said
   * nothing usable (a live stream, or no format). The pictures go with the
   * sound YouTube plays by default (Zajil then takes the video's own), or
   * with the sound of language `lang` when the user chose one (owner's
   * request, 2026-10-31), sent with it; `tracks` are the languages to
   * choose from, when there are several. YouTube's «default» is not the
   * video's own: it marks the dub in the viewer's language so (owner's
   * report, 2026-10-31: an Arabic dub chosen came as English), so the list
   * never says which is the original; Zajil knows, from yt-dlp.
   * @param {any} data @param {string | null} [lang]
   * @returns {{ title: string, duration: string, video: Choice[], audio: Choice[], tracks: Track[], lang: string } | null}
   * @typedef {{ name: string, detail: string, size: string, bytes?: number, send: { id?: string, video?: string, audio?: string, language?: string } }} Choice
   * @typedef {{ name: string, lang: string, main: boolean }} Track
   */
  function choicesOf(data, lang = null) {
    if (!data || typeof data !== "object" || data.live === true || !Array.isArray(data.formats)) return null;
    // Once each: YouTube lists an itag again for a "stable volume" sound
    // or a variant of it; the plain one comes first.
    const seen = new Set();
    const all = data.formats
      .slice(0, 200)
      .map(checked)
      .filter((/** @type {any} */ f) => f !== null && !f.drc && (f.kind !== "audio" || f.plain || f.codec === "AAC"))
      .filter((/** @type {any} */ f) => {
        // An itag once for each sound's language: a dub has its own.
        const key = `${f.itag}|${f.track?.lang ?? ""}`;
        return !seen.has(key) && seen.add(key);
      });
    // The sound that goes with a picture: the video's own track (not
    // another language's, not a "stable volume" one), the best of its kind.
    // A video whose sound has a language but no track marked the default
    // (one language only, often): every track counts as the video's own.
    const tracked = all.filter((/** @type {any} */ f) => f.kind === "audio" && f.track);
    const anyMain = tracked.some((/** @type {any} */ f) => f.track.main);
    const isMain = (/** @type {any} */ f) => !f.track || f.track.main || !anyMain;
    // A language is named only when there are several.
    const languages = new Set(tracked.map((/** @type {any} */ f) => f.track.name)).size;
    /** @type {Track[]} */
    const tracks = [];
    for (const f of tracked) {
      if (f.track.lang && !tracks.some((t) => t.lang === f.track.lang))
        tracks.push({ name: f.track.name || f.track.lang, lang: f.track.lang, main: isMain(f) });
    }
    tracks.sort((a, b) => Number(b.main) - Number(a.main));
    // The chosen language, if it is one of them; else YouTube's default.
    const dub = tracks.find((t) => t.lang === lang) ?? null;
    const ofSound = (/** @type {any} */ f) => (dub ? f.track?.lang === dub.lang : isMain(f));
    const sounds = all.filter((/** @type {any} */ f) => f.kind === "audio" && f.plain && ofSound(f));
    /** YouTube's usual one first (140, 251), else the best of its kind.
     * @param {string} ext @param {string} usual */
    const bestSound = (ext, usual) =>
      sounds.find((/** @type {any} */ f) => f.itag === usual) ??
      sounds.filter((/** @type {any} */ f) => f.ext === ext).sort((/** @type {any} */ a, /** @type {any} */ b) => (b.bitrate ?? 0) - (a.bitrate ?? 0))[0];
    const m4a = bestSound("m4a", "140");
    const opus = bestSound("webm", "251");

    /** @type {Choice[]} */
    const video = [];
    const pictures = all
      .filter((/** @type {any} */ f) => f.kind !== "audio")
      .sort(
        (/** @type {any} */ a, /** @type {any} */ b) =>
          (b.height ?? 0) - (a.height ?? 0) ||
          (b.fps ?? 0) - (a.fps ?? 0) ||
          CODEC_ORDER.indexOf(a.codec) - CODEC_ORDER.indexOf(b.codec),
      );
    for (const f of pictures) {
      // A picture with its own sound has one we do not know: not with a
      // chosen language.
      if (dub && f.kind === "both") continue;
      // A WebM picture goes with Opus; any other with AAC (else Opus: MKV).
      const sound = f.kind === "both" ? null : f.ext === "webm" ? (opus ?? m4a) : (m4a ?? opus);
      if (f.kind === "video" && !sound) continue;
      const ext = f.kind === "both" ? f.ext : f.ext === "mp4" && sound?.ext === "m4a" ? "mp4" : f.ext === "webm" && sound?.ext === "webm" ? "webm" : "mkv";
      const size = f.size !== undefined && (sound ? sound.size !== undefined : true) ? f.size + (sound?.size ?? 0) : undefined;
      const rate = f.bitrate !== undefined ? f.bitrate + (sound?.bitrate ?? 0) : undefined;
      video.push({
        name: `${ext.toUpperCase()} ${f.quality || (f.height ? `${f.height}p` : "")}`.trim(),
        detail: [f.codec, rateText(rate)].filter(Boolean).join(" · "),
        size: sizeText(size),
        bytes: size,
        send: sound ? { video: f.itag, audio: sound.itag, ...(dub ? { language: dub.lang } : {}) } : { id: f.itag },
      });
    }
    /** @type {Choice[]} */
    // The sounds of the chosen language only, when there are several.
    const audio = all
      .filter((/** @type {any} */ f) => f.kind === "audio" && (tracks.length < 2 || ofSound(f)))
      .sort((/** @type {any} */ a, /** @type {any} */ b) => Number(isMain(b)) - Number(isMain(a)) || (b.bitrate ?? 0) - (a.bitrate ?? 0))
      .map((/** @type {any} */ f) => ({
        name: `${f.ext.toUpperCase()} ${rateText(f.bitrate)}`.trim() + (f.track?.name && languages > 1 ? ` · ${f.track.name}` : ""),
        detail: f.codec,
        size: sizeText(f.size),
        bytes: f.size,
        send: dub && f.track?.lang ? { id: f.itag, language: f.track.lang } : { id: f.itag },
      }));
    if (!video.length && !audio.length) return null;
    return {
      title: text(data.title, 200),
      duration: durationText(num(data.seconds, 1e7)),
      video,
      audio,
      tracks: tracks.length > 1 ? tracks : [],
      // The chosen language; "" for the video's own, Zajil's choice.
      lang: dub?.lang ?? "",
    };
  }

  g.__zajilFormats = { choicesOf };
})();
