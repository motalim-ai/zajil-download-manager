// How a file name that mixes Arabic, Latin and numbers is shown (the rule
// of the approved M4 design, after the owner's review): the whole name is
// one isolate whose direction comes from its first strong letter, and the
// extension joins the name's last word and takes that word's direction:
// - a name that ends in Arabic keeps the extension in the normal flow, so
//   the dot stays against the word («تقرير … النهائي.pdf» → «pdf.النهائي»);
// - a name that ends in Latin text keeps that text and the extension as
//   one left-to-right unit («… (Part 2).mp4» shows «(Part 2).mp4» whole).

export interface NameParts {
  /// Shown in the outer isolate (dir="auto").
  head: string;
  /// Shown in an inner left-to-right isolate after `head`, or empty.
  tail: string;
}

// Right-to-left letters, not the Arabic-Indic digits and their separators
// (U+0660–U+066C, U+06F0–U+06F9), which the bidi algorithm orders as
// numbers, nor U+FEFF.
const RTL = /[\u0590-\u065F\u066D-\u06EF\u06FA-\u08FF\uFB1D-\uFDFF\uFE70-\uFEFC]/;
// Letters written left to right: Latin, Greek and Cyrillic.
const LATIN = /[A-Za-z\u00C0-\u024F\u0370-\u03FF\u0400-\u04FF]/;
const OPENING = /[(\[{"'«]/;
// Digits and joiners stuck to the first Latin letter belong to its word:
// «1080p», «3D», «12-Part».
const STUCK = /[0-9_.\-]/;
const JOINER = /[_.\-]/;

/// The direction a name box gives the name, by its first strong letter
/// (as `dir="auto"`, which looks through no isolate): what is drawn over
/// the box lines up with it.
export function nameDir(name: string): "rtl" | "ltr" {
  for (const c of name) {
    if (RTL.test(c)) return "rtl";
    if (LATIN.test(c)) return "ltr";
  }
  return "ltr";
}

/// A name for a text the page does not draw (a window's title, which the
/// system draws): FileName's two isolates as Unicode isolate marks, so the
/// system orders it as FileName does (M5.5).
export function isolatedName(name: string): string {
  const { head, tail } = nameParts(name);
  return `\u2068${head}${tail ? `\u2066${tail}\u2069` : ""}\u2069`;
}

export function nameParts(name: string): NameParts {
  const match = /^(.*?)((?:\.tar)?\.[A-Za-z0-9]{1,8})$/.exec(name);
  if (!match) return { head: name, tail: "" };
  const stem = match[1] ?? "";
  const ext = match[2] ?? "";
  let lastRtl = -1;
  for (let i = stem.length - 1; i >= 0; i--) {
    if (RTL.test(stem[i] ?? "")) {
      lastRtl = i;
      break;
    }
  }
  // All Latin, or ending in Arabic: the normal flow is right.
  if (lastRtl < 0) return { head: name, tail: "" };
  let firstLatin = -1;
  for (let i = lastRtl + 1; i < stem.length; i++) {
    if (LATIN.test(stem[i] ?? "")) {
      firstLatin = i;
      break;
    }
  }
  if (firstLatin < 0) return { head: name, tail: "" };
  let cut = firstLatin;
  while (cut > lastRtl + 1 && STUCK.test(stem[cut - 1] ?? "")) cut--;
  // The word starts at a digit, never at a joiner.
  while (cut < firstLatin && JOINER.test(stem[cut] ?? "")) cut++;
  while (cut > lastRtl + 1 && OPENING.test(stem[cut - 1] ?? "")) cut--;
  return { head: stem.slice(0, cut), tail: stem.slice(cut) + ext };
}
