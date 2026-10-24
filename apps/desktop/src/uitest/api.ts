// ui-test builds only: the calls of the UI tests, loaded by App.svelte
// behind `__UI_TEST__`, so a release build has none of them, not even their
// names (the owner's condition on #47; CI checks the release page and the
// release program for them).
import { invoke } from "@tauri-apps/api/core";
import { currentMonitor, getCurrentWindow } from "@tauri-apps/api/window";

type Rect = { x: number; y: number; width: number; height: number };

/// Where the page is on screen, in physical pixels (the capture script
/// takes exactly this rectangle), and the screen's work area, which the
/// rectangle must lie in (review of #47: on Windows it ran under the
/// taskbar). Needs the `ui-test` capability.
export async function windowRect(): Promise<Rect & { scale: number; screen: Rect | null }> {
  const w = getCurrentWindow();
  const [position, size, scale, monitor] = await Promise.all([w.innerPosition(), w.innerSize(), w.scaleFactor(), currentMonitor()]);
  const area = monitor?.workArea;
  const screen = area ? { x: area.position.x, y: area.position.y, width: area.size.width, height: area.size.height } : null;
  return { x: position.x, y: position.y, width: size.width, height: size.height, scale, screen };
}

/// The scene asked for with `--ui-test`, or null.
export const uiTestScenario = () => invoke<string | null>("ui_test_scenario");

/// What the page reports, written next to the screenshot.
export const uiTestReport = (report: unknown) => invoke<void>("ui_test_report", { report });
