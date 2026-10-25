// The table's columns (M5.5, the design of #83): shown or hidden, their
// width and their order, as the user leaves them. Kept per viewer in
// `localStorage` (a convenience: without it, the defaults).

export type ColumnId = "name" | "size" | "status" | "left" | "speed" | "last" | "desc";

/** Every column, in the design's order, with its name's key and its width
 * (`null`: the rest of the table). The name cannot be hidden. */
export const COLUMNS: { id: ColumnId; key: string; width: number | null }[] = [
  { id: "name", key: "col-name", width: null },
  { id: "size", key: "col-size", width: 100 },
  { id: "status", key: "col-status", width: 128 },
  { id: "left", key: "col-left", width: 84 },
  { id: "speed", key: "col-speed", width: 112 },
  { id: "last", key: "col-last-try", width: 88 },
  { id: "desc", key: "col-description", width: 88 },
];

export const MIN_WIDTH = 56;
export const MAX_WIDTH = 600;

export type Columns = { order: ColumnId[]; hidden: ColumnId[]; widths: Partial<Record<ColumnId, number>> };

export const DEFAULT: Columns = { order: COLUMNS.map((c) => c.id), hidden: [], widths: {} };

const KEY = "zajil.columns.v1";
const ids = new Set<string>(COLUMNS.map((c) => c.id));

/** What was kept, checked: unknown or repeated columns are dropped, missing
 * ones come back at the end, the name is never hidden, and widths stay in
 * bounds. Anything unreadable gives the defaults. */
export function clean(value: unknown): Columns {
  if (typeof value !== "object" || value === null) return { ...DEFAULT, order: [...DEFAULT.order] };
  const v = value as Partial<Record<keyof Columns, unknown>>;
  const order: ColumnId[] = [];
  for (const id of Array.isArray(v.order) ? v.order : []) {
    if (typeof id === "string" && ids.has(id) && !order.includes(id as ColumnId)) order.push(id as ColumnId);
  }
  for (const c of COLUMNS) if (!order.includes(c.id)) order.push(c.id);
  const hidden = (Array.isArray(v.hidden) ? v.hidden : []).filter(
    (id): id is ColumnId => typeof id === "string" && ids.has(id) && id !== "name",
  );
  const widths: Partial<Record<ColumnId, number>> = {};
  if (typeof v.widths === "object" && v.widths !== null) {
    for (const [id, w] of Object.entries(v.widths)) {
      if (ids.has(id) && id !== "name" && typeof w === "number" && Number.isFinite(w)) widths[id as ColumnId] = clampWidth(w);
    }
  }
  return { order, hidden: [...new Set(hidden)], widths };
}

export const clampWidth = (w: number): number => Math.round(Math.min(MAX_WIDTH, Math.max(MIN_WIDTH, w)));

export function load(storage: Pick<Storage, "getItem"> | undefined = globalThis.localStorage): Columns {
  try {
    const text = storage?.getItem(KEY);
    return clean(text ? JSON.parse(text) : null);
  } catch {
    return clean(null);
  }
}

export function save(columns: Columns, storage: Pick<Storage, "setItem"> | undefined = globalThis.localStorage): void {
  try {
    storage?.setItem(KEY, JSON.stringify(columns));
  } catch {
    // A private window or blocked storage: the columns are not kept.
  }
}

/** `columns` with column `id` moved to where `before` is (or the end). */
export function move(columns: Columns, id: ColumnId, before: ColumnId | null): Columns {
  if (id === before) return columns;
  const order = columns.order.filter((c) => c !== id);
  const at = before === null ? order.length : order.indexOf(before);
  order.splice(at < 0 ? order.length : at, 0, id);
  return { ...columns, order };
}

/** `columns` with column `id` shown or hidden (the name stays). */
export function toggle(columns: Columns, id: ColumnId): Columns {
  if (id === "name") return columns;
  const hidden = columns.hidden.includes(id) ? columns.hidden.filter((c) => c !== id) : [...columns.hidden, id];
  return { ...columns, hidden };
}

/** The columns to draw, in order. */
export function visible(columns: Columns): { id: ColumnId; key: string; width: number | null }[] {
  return columns.order
    .filter((id) => !columns.hidden.includes(id))
    .map((id) => {
      const c = COLUMNS.find((x) => x.id === id)!;
      return { ...c, width: c.width === null ? null : (columns.widths[id] ?? c.width) };
    });
}
