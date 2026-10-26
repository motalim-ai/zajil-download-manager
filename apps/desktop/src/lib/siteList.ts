// The per-site connection limits as the settings screen edits them (#165):
// a site as the user types it, made into the settings file's form, and the
// list checked as the file is (SettingsFile::check), so a mistake is said
// in the user's language before anything is saved.

export type SiteRow = { site: string; connections: number };

/** The most connections at once to one site (zajil_core::MAX_CONNECTIONS). */
export const MAX_CONNECTIONS = 32;
/** The most sites with a limit (zajil_core::sites::MAX_SITES). */
export const MAX_SITES = 100;

/** `text` (`https://Example.com/path`, `example.com:8080`, `مثال.com`) as
 * the file wants it: the host alone, lowercase, in its xn-- form; null if
 * it is not one. */
export function normalizeSite(text: string): string | null {
  const trimmed = text.trim();
  if (!trimmed || /\s/.test(trimmed)) return null;
  let host: string;
  try {
    host = new URL(trimmed.includes("://") ? trimmed : `http://${trimmed}`).hostname;
  } catch {
    return null;
  }
  host = host.replace(/\.+$/, "").toLowerCase();
  if (!host || host.startsWith(".") || !/^[a-z0-9.\-:[\]]+$/.test(host)) return null;
  return host;
}

export type Problem = { key: "sites-bad-site" | "sites-twice" | "sites-bad-number" | "sites-too-many"; site: string };

/** The list ready to save, or the first problem in it. */
export function checkList(rows: SiteRow[]): { list: SiteRow[] } | { problem: Problem } {
  if (rows.length > MAX_SITES) return { problem: { key: "sites-too-many", site: "" } };
  const list: SiteRow[] = [];
  for (const row of rows) {
    const site = normalizeSite(row.site);
    if (site === null) return { problem: { key: "sites-bad-site", site: row.site.trim() } };
    if (list.some((r) => r.site === site)) return { problem: { key: "sites-twice", site } };
    const n = Number(row.connections);
    if (!Number.isInteger(n) || n < 1 || n > MAX_CONNECTIONS) return { problem: { key: "sites-bad-number", site } };
    list.push({ site, connections: n });
  }
  return { list };
}
