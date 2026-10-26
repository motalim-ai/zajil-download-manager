// ui-test builds only (main.ts loads it behind `__UI_TEST__`): every window
// a button opens, opened through the page's own calls, and each window's
// page says it ran (`src-tauri/tests/windows_open.rs`). On Windows a window
// opened by a command that is not async kept a blank page (#226): its frame
// came, its page never did.
import { invoke } from "@tauri-apps/api/core";
import { listen } from "@tauri-apps/api/event";
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
  // Each as its button opens it, by its label; the next once its page
  // ran (20 s at most): «Download file info» closes «Add link». A refusal
  // is written down, not thrown.
  const loaded = new Set<string>();
  await listen<string>("ui-test-loaded", (e) => loaded.add(e.payload));
  const opens: [string, () => Promise<unknown>][] = [
    ["add-link", () => api.openAddLink(null, false, "Zajil")],
    ["add-info", () => api.openAddInfo("http://127.0.0.1:9/file.bin", "Zajil")],
    ["progress-1", () => api.openProgress(1, "Zajil")],
    ["progress-2", () => api.openStartedProgress(2, "Zajil")],
    ["schedule", () => scheduleApi.open("Zajil")],
    ["batch", () => api.openBatch(null)],
    ["grab", () => api.openGrab(null)],
    ["video", () => api.openVideo(null)],
    ["drop", () => api.setDropBox(true)],
  ];
  for (const [label, open] of opens) {
    const refused = await open().then(
      () => false,
      (e) => invoke("ui_test_loaded", { label: `refused-${label}`, why: String(e) }).then(() => true),
    );
    for (let i = 0; !refused && i < 200 && !loaded.has(label); i++) await new Promise((r) => setTimeout(r, 100));
  }
}
