// «Video quality»'s choices (M6, #149), from the formats a page offers:
// each picture height once, best first; a picture-only format is merged
// with the sound of the engine's best choice (else the largest m4a);
// «Sound only (m4a)» last. The engine's best choice is always in the
// list, and is the one picked at first.
import type { ChoiceView } from "./bindings/ChoiceView";
import type { FormatView } from "./bindings/FormatView";

export type Option = {
  choice: ChoiceView;
  /** Picture height (0: sound only). */
  height: number;
  ext: string;
  /** The picture's codec (`H.264`, `VP9`…), if the site says it. */
  codec: string | null;
  fps: number | null;
  /** Bytes, or nothing if the site gives no size. */
  size: number | null;
  /** «Sound only». */
  sound?: boolean;
};

/** One of the sounds a page offers in several languages: the video's own,
 * or a dubbed one (owner's request, 2026-10-31: «what if I want the dub I
 * hear?»). */
export type Track = { language: string; original: boolean };

const ofTrack = (f: FormatView, t: Track) => f.kind === "audio" && (f.language ?? "") === t.language && f.original === t.original;

/** The sounds to choose from, the video's own first; none when there is
 * only one. */
export function tracks(formats: FormatView[]): Track[] {
  const found: Track[] = [];
  for (const f of formats) {
    if (f.kind !== "audio" || !f.language) continue;
    const t = { language: f.language, original: f.original };
    if (!found.some((o) => o.language === t.language && o.original === t.original)) found.push(t);
  }
  found.sort((a, b) => Number(b.original) - Number(a.original));
  return found.length > 1 ? found : [];
}

/** The sound merged with each picture: of the chosen track if any, else
 * the engine's, else the largest m4a, among the video's own sounds when
 * the page marks them (a dubbed one only when there is nothing else;
 * users' reports, 2026-10-31). */
function bestAudio(formats: FormatView[], best: ChoiceView | null, track: Track | null): FormatView | undefined {
  const sounds = formats.filter((f) => f.kind === "audio");
  const chosen = track ? sounds.filter((f) => ofTrack(f, track)) : [];
  const own = sounds.filter((f) => f.original);
  const audio = chosen.length > 0 ? chosen : own.length > 0 ? own : sounds;
  const largest = (list: FormatView[]) => list.reduce<FormatView | undefined>((a, f) => (!a || (f.size ?? 0) > (a.size ?? 0) ? f : a), undefined);
  return audio.find((f) => best?.kind === "merge" && f.id === best.audio) ?? largest(audio.filter((f) => f.ext === "m4a")) ?? largest(audio);
}

export function options(formats: FormatView[], best: ChoiceView | null = null, track: Track | null = null): Option[] {
  const audio = bestAudio(formats, best, track);
  // A dubbed sound: a picture with its sound in one format has the video's
  // own, so only merges are offered, and «sound only» is that sound.
  const dubbed = track !== null && !track.original && audio !== undefined && ofTrack(audio, track);
  const list: Option[] = [];
  for (const f of formats) {
    if (f.kind === "both" && !dubbed) list.push({ choice: { kind: "format", id: f.id }, height: f.height ?? 0, ext: f.ext, codec: f.codec ?? null, fps: f.fps, size: f.size });
    else if (f.kind === "video" && audio)
      list.push({
        choice: { kind: "merge", video: f.id, audio: audio.id },
        height: f.height ?? 0,
        ext: f.ext,
        codec: f.codec ?? null,
        fps: f.fps,
        size: f.size !== null && audio.size !== null ? f.size + audio.size : null,
      });
  }
  // Each height and kind of file once: the engine's best, else the one
  // with the size, else the first.
  const isBest = (o: Option) => best !== null && same(o.choice, best);
  const seen = new Map<string, Option>();
  for (const o of list) {
    const key = `${o.height}-${o.ext}`;
    const had = seen.get(key);
    if (!had || isBest(o) || (!isBest(had) && had.size === null && o.size !== null)) seen.set(key, o);
  }
  const sorted = [...seen.values()].sort((a, b) => b.height - a.height || (b.size ?? 0) - (a.size ?? 0));
  if (formats.some((f) => f.kind === "audio" || f.kind === "both"))
    sorted.push({
      choice: dubbed && audio ? { kind: "format", id: audio.id } : { kind: "audio_only" },
      height: 0,
      ext: dubbed && audio ? audio.ext : "m4a",
      codec: null,
      fps: null,
      size: audio?.size ?? null,
      sound: true,
    });
  return sorted;
}

export const same = (a: ChoiceView, b: ChoiceView): boolean => JSON.stringify(a) === JSON.stringify(b);

/** Whether QuickTime (macOS's own player) cannot play this picture:
 * VP9, VP8 or AV1 (owner's decision, 2026-10-30: say so on the Mac). */
export const notForQuickTime = (o: Option): boolean => o.codec === "VP9" || o.codec === "VP8" || o.codec === "AV1";

/** The option picked at first: the engine's best, else the first. */
export function picked(list: Option[], best: ChoiceView | null): number {
  const at = best ? list.findIndex((o) => same(o.choice, best)) : -1;
  return at >= 0 ? at : 0;
}

/** The file's name: the title, without what a file name cannot hold
 * (the engine cleans it again). No extension: the engine adds the one
 * yt-dlp's file has, which only it knows (a merge may give mp4 or mkv). */
export function fileName(title: string): string {
  return title.replace(/[\\/:*?"<>|\u0000-\u001f]+/g, " ").replace(/\s+/g, " ").trim() || "video";
}
