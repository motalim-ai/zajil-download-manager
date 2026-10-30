// The list's order (owner's request, 2026-10-30): the newest download
// first. By when it joined the list (RFC 3339 in UTC sorts as text); a
// download without that time (a video saved before 1.1.3) after those with
// one, in the order it came.
type Added = { added: string | null };

export function newestFirst<T extends Added>(list: T[]): T[] {
  return list
    .map((d, at) => ({ d, at }))
    .sort((a, b) => {
      if (a.d.added && b.d.added && a.d.added !== b.d.added) return a.d.added < b.d.added ? 1 : -1;
      if (!a.d.added !== !b.d.added) return a.d.added ? -1 : 1;
      return b.at - a.at;
    })
    .map(({ d }) => d);
}
