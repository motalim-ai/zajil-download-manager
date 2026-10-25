// "Refresh the download link" (M5): what the page says when the program
// refused a new link (`RefreshProblem`, from Rust).
import { size } from "./format";
import type { Digits, Lang } from "./i18n.svelte";
import type { RefreshProblem } from "./bindings/RefreshProblem";

type T = (key: string, args?: Record<string, string>) => string;

/// The text for `problem`, and the English detail from the program when
/// there is one (shown apart, as the add dialog shows the server's words).
export function refreshText(
  t: T,
  lang: Lang,
  digits: Digits,
  problem: RefreshProblem,
): { text: string; detail: string | null } {
  switch (problem.kind) {
    case "not_stopped":
      return { text: t("refresh-not-stopped"), detail: null };
    case "done":
      return { text: t("refresh-done"), detail: null };
    case "bad_link":
      return { text: t("refresh-bad-link"), detail: problem.reason };
    case "failed":
      return { text: t("refresh-failed"), detail: problem.error };
    case "not_the_same_file": {
      const shown = (n: number | null) => (n === null ? t("details-unknown") : size(lang, digits, n));
      switch (problem.why) {
        case "size":
          return { text: t("refresh-size", { theirs: shown(problem.theirs), ours: shown(problem.ours) }), detail: null };
        case "version":
          return { text: t("refresh-version"), detail: null };
        case "validator_kind":
          return { text: t("refresh-validator-kind"), detail: null };
        case "new_no_validator":
          return { text: t("refresh-new-no-validator"), detail: null };
        case "no_validator":
          return { text: t("refresh-no-validator"), detail: null };
      }
    }
  }
}

/// What a refused call carries: the program's `RefreshProblem`, or
/// anything else (a call that never reached it) as a failure.
export function asRefreshProblem(e: unknown): RefreshProblem {
  if (typeof e === "object" && e !== null && typeof (e as { kind?: unknown }).kind === "string") {
    return e as RefreshProblem;
  }
  return { kind: "failed", error: String(e) };
}
