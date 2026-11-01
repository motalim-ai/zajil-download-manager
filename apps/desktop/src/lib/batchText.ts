// «Batch download» (M7): why a pattern was refused, in the user's words:
// what is wrong, and where in the link when that is known.
import type { BatchProblem } from "./bindings/BatchProblem";

type Say = (key: string, args?: Record<string, string | number>) => string;

/** The lines that say why `problem` was refused. */
export function problemText(say: Say, problem: BatchProblem): string[] {
  const what =
    problem.kind === "too_many"
      ? say("batch-error-too_many", { max: problem.max ?? 0 })
      : problem.kind === "not_http"
        ? say("batch-error-not_http", { http: "http://", https: "https://" })
        : problem.kind === "video_page"
          ? say("batch-error-video_page", { count: problem.count ?? 0 })
          : say(`batch-error-${problem.kind}`);
  return problem.at !== null ? [what, say("batch-error-at", { at: problem.at })] : [what];
}

/** Whether `text` is a refusal from Rust (the preview's error), not a
 * failure to reach it. */
export function isProblem(e: unknown): e is BatchProblem {
  return typeof e === "object" && e !== null && "kind" in e && typeof (e as { kind: unknown }).kind === "string";
}
