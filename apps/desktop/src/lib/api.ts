// The commands of the Rust side (src-tauri/src/lib.rs). The types are
// generated from Rust (ts-rs), so both sides agree.
import { invoke } from "@tauri-apps/api/core";
import { listen, type UnlistenFn } from "@tauri-apps/api/event";
import { getCurrentWindow } from "@tauri-apps/api/window";
import type { Snapshot } from "./bindings/Snapshot";

/// Where the page is on screen, in physical pixels, for the screenshots of
/// the UI tests (the capture script takes exactly this rectangle).
export async function windowRect(): Promise<{ x: number; y: number; width: number; height: number; scale: number }> {
  const w = getCurrentWindow();
  const [position, size, scale] = await Promise.all([w.innerPosition(), w.innerSize(), w.scaleFactor()]);
  return { x: position.x, y: position.y, width: size.width, height: size.height, scale };
}

export const api = {
  snapshot: () => invoke<Snapshot>("snapshot"),
  pause: (ids: number[]) => invoke<void>("pause", { ids }),
  resume: (ids: number[]) => invoke<void>("resume", { ids }),
  remove: (ids: number[]) => invoke<void>("remove", { ids }),
  onChange: (handler: () => void): Promise<UnlistenFn> => listen("downloads-changed", handler),
  // Only in a `ui-test` build: in a release build these names are not even
  // in the code (the commands do not exist there, and CI checks both).
  uiTestScenario: (): Promise<string | null> =>
    __UI_TEST__ ? invoke<string | null>("ui_test_scenario") : Promise.resolve(null),
  uiTestReport: (report: unknown): Promise<void> =>
    __UI_TEST__ ? invoke<void>("ui_test_report", { report }) : Promise.resolve(),
};
