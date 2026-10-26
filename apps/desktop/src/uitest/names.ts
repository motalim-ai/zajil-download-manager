// ui-test builds only: is every file name of a scene drawn as FileName
// draws it (M5.5 criterion: mixed names right in every window)? A name
// drawn any other way (plain text, or a name box showing its own text)
// orders a mixed name as one paragraph, the rendering the owner rejected
// in M4; scripts/ui-shots.mjs fails on it.

const RTL = /[֐-ࣿיִ-﷿ﹰ-ﻼ]/;
const LATIN = /[A-Za-z]/;

export const isMixed = (name: string) => RTL.test(name) && LATIN.test(name);

export type NamesSeen = {
  /** Mixed names drawn by FileName. */
  drawn: number;
  /** Mixed names drawn otherwise, with where. */
  raw: string[];
};

/** Looks at what `root` shows of `names` (the scene's file names). */
export function checkNames(root: HTMLElement, names: string[]): NamesSeen {
  const mixed = [...new Set(names.filter(isMixed))];
  const raw: string[] = [];
  let drawn = 0;
  for (const el of root.querySelectorAll<HTMLElement>("bdi.fn")) if (isMixed(el.title)) drawn++;
  // Plain text: FileName splits a mixed name in two, so a whole one in a
  // single piece of text was drawn by something else.
  const walker = document.createTreeWalker(root, NodeFilter.SHOW_TEXT);
  for (let node = walker.nextNode(); node; node = walker.nextNode()) {
    const text = node.textContent ?? "";
    const parent = node.parentElement;
    // Drawn by FileName; only for screen readers; or a link, whose path
    // may hold the name and which is shown left to right whole.
    if (!parent || parent.closest("bdi.fn, .visually-hidden") || /^\s*[a-z]+:\/\//i.test(text)) continue;
    for (const name of mixed) if (text.includes(name)) raw.push(`${name} in <${parent.tagName.toLowerCase()} class="${parent.className}">`);
  }
  // A name box: its name is drawn over it by FileName while not edited.
  for (const input of root.querySelectorAll<HTMLInputElement>("input[id$='-name']")) {
    if (!isMixed(input.value)) continue;
    const over = input.parentElement?.querySelector<HTMLElement>("bdi.fn");
    if (over?.title === input.value && getComputedStyle(input).color === "rgba(0, 0, 0, 0)") continue;
    raw.push(`${input.value} in the box #${input.id}`);
  }
  return { drawn, raw };
}
