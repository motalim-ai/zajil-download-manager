// The toolbar as the user leaves it (M5.5, an IDM touch): big or small
// buttons, which buttons, and in what order. Kept per viewer in
// `localStorage` (a convenience: without it, the design's toolbar).
import type { ToolAction } from "./components/Toolbar.svelte";

/** Every button, in the design's order, with the group it belongs to (a
 * line goes between two groups). */
export const BUTTONS: { action: ToolAction; group: number }[] = [
  { action: "add", group: 0 },
  { action: "resume", group: 1 },
  { action: "pause", group: 1 },
  { action: "pause-all", group: 1 },
  { action: "resume-all", group: 1 },
  { action: "delete", group: 2 },
  { action: "delete-done", group: 2 },
  { action: "settings", group: 3 },
  { action: "schedule", group: 3 },
  { action: "run-queue", group: 4 },
  { action: "halt-queue", group: 4 },
];

export type ToolbarPrefs = { order: ToolAction[]; hidden: ToolAction[]; small: boolean };

export const DEFAULT: ToolbarPrefs = { order: BUTTONS.map((b) => b.action), hidden: [], small: false };

const KEY = "zajil.toolbar.v1";
const known = new Set<string>(BUTTONS.map((b) => b.action));

/** What was kept, checked: unknown or repeated buttons dropped, missing
 * ones back after the one before them in the design (a new button shows
 * where it belongs). Anything unreadable: the defaults. */
export function clean(value: unknown): ToolbarPrefs {
  if (typeof value !== "object" || value === null) return { ...DEFAULT, order: [...DEFAULT.order] };
  const v = value as Partial<Record<keyof ToolbarPrefs, unknown>>;
  const order: ToolAction[] = [];
  for (const a of Array.isArray(v.order) ? v.order : []) {
    if (typeof a === "string" && known.has(a) && !order.includes(a as ToolAction)) order.push(a as ToolAction);
  }
  BUTTONS.forEach((b, i) => {
    if (order.includes(b.action)) return;
    const previous = BUTTONS[i - 1];
    const before = previous ? order.indexOf(previous.action) : -1;
    order.splice(before + 1, 0, b.action);
  });
  const hidden = [...new Set((Array.isArray(v.hidden) ? v.hidden : []).filter((a): a is ToolAction => typeof a === "string" && known.has(a)))];
  return { order, hidden, small: v.small === true };
}

export function load(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): ToolbarPrefs {
  try {
    const text = storage?.getItem(KEY);
    return clean(text ? JSON.parse(text) : null);
  } catch {
    return clean(null);
  }
}

export function save(prefs: ToolbarPrefs, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  try {
    storage?.setItem(KEY, JSON.stringify(prefs));
  } catch {
    // Not kept.
  }
}

/** `prefs` with `action` one place earlier (`-1`) or later (`1`). */
export function shift(prefs: ToolbarPrefs, action: ToolAction, by: -1 | 1): ToolbarPrefs {
  const order = [...prefs.order];
  const at = order.indexOf(action);
  const to = at + by;
  if (at < 0 || to < 0 || to >= order.length) return prefs;
  [order[at], order[to]] = [order[to]!, order[at]!];
  return { ...prefs, order };
}

export function toggle(prefs: ToolbarPrefs, action: ToolAction): ToolbarPrefs {
  const hidden = prefs.hidden.includes(action) ? prefs.hidden.filter((a) => a !== action) : [...prefs.hidden, action];
  return { ...prefs, hidden };
}

/** The buttons shown, in order, each with whether a line comes before it
 * (its group differs from the one before). */
export function shown(prefs: ToolbarPrefs): { action: ToolAction; line: boolean }[] {
  const group = (a: ToolAction) => BUTTONS.find((b) => b.action === a)!.group;
  const list = prefs.order.filter((a) => !prefs.hidden.includes(a));
  return list.map((action, i) => ({ action, line: i > 0 && group(action) !== group(list[i - 1]!) }));
}
