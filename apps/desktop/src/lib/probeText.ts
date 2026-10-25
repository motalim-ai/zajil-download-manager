// What «Download file info» says of the file (M5.5, and the owner's test
// with a Google Drive link): its size in words when the server gives none
// («الحجم غير معروف», not «?»), and the sentence on its connections.
import { size } from "./format";
import type { Digits, Lang } from "./i18n.svelte";
import type { ProbeView } from "./bindings/ProbeView";

type T = (key: string, args?: Record<string, string | number>) => string;

/** The size, or «unknown» in words. */
export function sizeText(t: T, lang: Lang, digits: Digits, probe: Pick<ProbeView, "size">): string {
  return probe.size === null ? t("add-size-unknown") : size(lang, digits, probe.size);
}

/** The sentence on how it downloads, starting with its size. */
export function probeSentence(t: T, lang: Lang, digits: Digits, probe: Pick<ProbeView, "size" | "resumable" | "ranges">): string {
  const key = probe.resumable ? "add-probe-ranges" : probe.ranges ? "add-probe-unsure" : "add-probe-single";
  return t(key, { size: sizeText(t, lang, digits, probe) });
}
