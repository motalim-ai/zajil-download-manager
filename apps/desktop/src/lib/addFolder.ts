// «Download file info»: the folder follows the category the user picks,
// until the user picks a folder by hand (owner's request for 0.1).
import type { CategoryView } from "./bindings/CategoryView";

/** Where category `id` saves: its own folder, else the downloads folder. */
export function categoryFolder(categories: CategoryView[], id: string | null, downloadFolder: string): string {
  return (id ? categories.find((c) => c.id === id)?.folder : null) ?? downloadFolder;
}

/** The folder shown, and the one the download is added with (`null`: by
 * its category, as the engine decides). */
export function chosenFolder(opts: {
  categories: CategoryView[];
  detected: string | null;
  picked: string | null;
  byHand: string | null;
  detectedFolder: string;
  downloadFolder: string;
}): { shown: string; request: string | null } {
  if (opts.byHand !== null) return { shown: opts.byHand, request: opts.byHand };
  if (opts.picked === opts.detected) return { shown: opts.detectedFolder, request: null };
  const shown = categoryFolder(opts.categories, opts.picked, opts.downloadFolder);
  return { shown, request: shown };
}

/** «Save as» (the owner's notes on 1.0): the folder and the name in one
 * path, with the folder's own separator. */
export function savePath(folder: string, name: string): string {
  if (!folder) return name;
  const sep = folder.includes("\\") && !folder.includes("/") ? "\\" : "/";
  return (folder.endsWith(sep) ? folder : folder + sep) + name;
}

/** What the user typed in «Save as»: its last part is the name (empty: the
 * server's), the rest the folder, `null` when it is the shown one or there
 * is none. */
export function typedPath(text: string, shown: string): { folder: string | null; name: string } {
  const trimmed = text.trim();
  const at = Math.max(trimmed.lastIndexOf("/"), trimmed.lastIndexOf("\\"));
  const name = trimmed.slice(at + 1).trim();
  if (at <= 0) return { folder: null, name };
  const dir = trimmed.slice(0, at);
  return { folder: dir === shown.replace(/[\\/]+$/, "") ? null : dir, name };
}
