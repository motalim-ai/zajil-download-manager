// What opening a download says when it cannot (#98).

/** The text of an error from `openDownload`: the codes for a finished file
 * or a folder that is not there any more, in the user's language; any
 * other error as it is. */
export function openError(e: unknown, t: (key: string) => string): string {
  const said = String(e);
  if (said === "no-file") return t("open-no-file");
  if (said === "no-folder") return t("open-no-folder");
  return said;
}
