// What a double-click on a finished download does (the owner's request:
// a setting, as in other download managers). Kept by the page, as the
// columns are; one still on its way always shows its progress.

export type DoubleClick = "open" | "folder" | "details";

const KEY = "zajil.double-click";

export function loadDoubleClick(): DoubleClick {
  try {
    const kept = localStorage.getItem(KEY);
    if (kept === "folder" || kept === "details") return kept;
  } catch {
    // The default.
  }
  return "open";
}

export function saveDoubleClick(choice: DoubleClick): void {
  try {
    localStorage.setItem(KEY, choice);
  } catch {
    // Not kept: the default next time.
  }
}
