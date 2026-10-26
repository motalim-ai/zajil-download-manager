// ui-test builds only (main.ts loads it behind `__UI_TEST__`): every window
// a button opens, opened through the page's own calls, and each window's
// page says it ran (`src-tauri/tests/windows_open.rs`). On Windows a window
// opened by a command that is not async kept a blank page (#226): its frame
// came, its page never did.
import { invoke } from "@tauri-apps/api/core";
import { getCurrentWindow } from "@tauri-apps/api/window";
import { api } from "../lib/api";
import { scheduleApi } from "../lib/scheduleApi";

/** Run with `--ui-test-windows`: the main window opens them all, one after
 * the other, and every other window says its page ran. */
export async function run(win: string | null): Promise<void> {
  if (!(await invoke<boolean>("ui_test_windows").catch(() => false))) return;
  if (win !== null) {
    await invoke("ui_test_loaded", { label: getCurrentWindow().label });
    return;
  }
  // Each as its button opens it; a refusal is written down, not thrown.
  const opens: [string, () => Promise<unknown>][] = [
    ["add-link", () => api.openAddLink(null, false, "Zajil")],
    ["add-info", () => api.openAddInfo("http://127.0.0.1:9/file.bin", "Zajil")],
    ["progress", () => api.openProgress(1, "Zajil")],
    ["started-progress", () => api.openStartedProgress(2, "Zajil")],
    ["schedule", () => scheduleApi.open("Zajil")],
    ["batch", () => api.openBatch(null)],
    ["grab", () => api.openGrab(null)],
    ["video", () => api.openVideo(null)],
    ["drop", () => api.setDropBox(true)],
  ];
  for (const [what, open] of opens) {
    await open().catch((e) => invoke("ui_test_loaded", { label: `refused-${what}`, why: String(e) }));
  }
}
