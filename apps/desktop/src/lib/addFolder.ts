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
