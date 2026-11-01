// A progress tick (`downloads-progress`, twice a second): only the running
// downloads' rows come, and only they change in the list (the owner's
// choice, after measuring the list's cost: the whole list came before).

type Row = { id: number };

/** Puts each of `rows` in place of its own in `list`. `false`, and nothing
 * changed, when one is not in the list: the whole list must be read again. */
export function patchRows<T extends Row>(list: T[], rows: T[]): boolean {
  const at = new Map(list.map((d, i) => [d.id, i]));
  if (rows.some((r) => !at.has(r.id))) return false;
  for (const r of rows) list[at.get(r.id)!] = r;
  return true;
}
