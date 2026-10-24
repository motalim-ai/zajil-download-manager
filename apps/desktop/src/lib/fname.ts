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

const RTL = /[֐-ࣿיִ-﷿ﹰ-﻿]/;
const LATIN = /[A-Za-zÀ-ɏ]/;
const OPENING = /[(\[{"'«]/;

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
  while (cut > lastRtl + 1 && OPENING.test(stem[cut - 1] ?? "")) cut--;
  return { head: stem.slice(0, cut), tail: stem.slice(cut) + ext };
}
