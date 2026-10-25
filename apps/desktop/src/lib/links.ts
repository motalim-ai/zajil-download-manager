// The link in what is dropped on the drop box (M5.5): the browser's link
// list (`text/uri-list`, comments starting with «#») or any text, the
// first http or https link in it, without the punctuation that ends a
// sentence around it.
const LINK = /\bhttps?:\/\/[^\s<>"'«»]+/i;

export function firstLink(...texts: string[]): string | null {
  for (const text of texts) {
    for (const line of text.split(/\r?\n/)) {
      if (line.startsWith("#")) continue;
      const found = LINK.exec(line)?.[0];
      if (found) return found.replace(/[.,;:!?)\]}،؛]+$/u, "");
    }
  }
  return null;
}
