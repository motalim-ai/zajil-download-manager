// What a category is called on screen (#58).

/** The built-in categories: their names are translated until the user
 * renames them. */
export const BUILT_IN = ["programs", "video", "music", "documents", "archives"];

/** A category's name as shown: the user's, else a built-in category's
 * translation, else its id. A category the user added and left without a
 * name showed the translation key (`category-custom-1`) instead (#58). */
export function categoryName(id: string, name: string | null | undefined, t: (key: string) => string): string {
  if (name) return name;
  return BUILT_IN.includes(id) ? t(`category-${id}`) : id;
}
