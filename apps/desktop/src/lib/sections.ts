// The sections under «All downloads» in the tree (the owner's notes on
// 1.0, as IDM's): the unfinished, finished and waiting downloads.

export type Section = "unfinished" | "finished" | "waiting";

/** A download of `status` is in section `section`. */
export function inSection(section: Section, status: string): boolean {
  if (section === "finished") return status === "done";
  if (section === "waiting") return status === "waiting";
  return status !== "done";
}
