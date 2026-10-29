import { mount } from "svelte";
import "./styles/app.css";
import App from "./App.svelte";
import LinkWindow from "./windows/LinkWindow.svelte";
import InfoWindow from "./windows/InfoWindow.svelte";
import ProgressWindow from "./windows/ProgressWindow.svelte";
import ScheduleWindow from "./windows/ScheduleWindow.svelte";
import CompleteWindow from "./windows/CompleteWindow.svelte";
import DropBox from "./windows/DropBox.svelte";
import VideoWindow from "./windows/VideoWindow.svelte";
import BatchWindow from "./windows/BatchWindow.svelte";
import GrabWindow from "./windows/GrabWindow.svelte";
import SettingsWindow from "./windows/SettingsWindow.svelte";
import { completeApi, loadShow } from "./lib/completeApi";
import { api } from "./lib/api";
import { loadSettings, startWindow } from "./lib/windowStart";
import type { Role } from "./lib/bindings/Role";

// One page for every window (M5.5): `?win=` says which one this is.
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const asked = new URLSearchParams(location.search).get("win");

/** Draws window `win` (`null`: the main one). */
function draw(win: string | null) {
  // Rust learns at each start whether «Download complete» shows (the user's
  // choice is kept by the page).
  if (win === null) void completeApi.setShow(loadShow()).catch(() => {});
  const progress = /^progress-(\d+)$/.exec(win ?? "");
  // ui-test builds only: M5.5's end-to-end test drives each window
  // (`src/uitest/e2e.ts`); a release build has none of it.
  if (__UI_TEST__) void import("./uitest/e2e").then((test) => test.drive(win));
  // ui-test builds only: the test that opens every window
  // (`src/uitest/windows.ts`); a release build has none of it.
  if (__UI_TEST__) void import("./uitest/windows").then((test) => test.run(win));
  if (progress) mount(ProgressWindow, { target: target!, props: { id: Number(progress[1]) } });
  else
    mount(
      win === "add-link" ? LinkWindow : win === "add-info" ? InfoWindow : win === "schedule" ? ScheduleWindow : win === "complete" ? CompleteWindow : win === "drop" ? DropBox : win === "video" ? VideoWindow : win === "batch" ? BatchWindow : win === "grab" ? GrabWindow : win === "settings" ? SettingsWindow : App,
      { target: target! },
    );
  // Drawn (timed with `ZAJIL_MEASURE=1`): at once, not at the next frame,
  // which a window behind another app may never get.
  void api.windowPainted().catch(() => {});
}

// A window made beforehand, hidden (`windows.rs`): its page loads and its
// settings are read now; it is drawn when it takes a role and shows.
if (asked === "spare") {
  void startWindow();
  let drawn = false;
  const take = (role: Role | null) => {
    if (drawn || !role) return;
    drawn = true;
    void loadSettings();
    draw(role.page);
  };
  void api.onWindowRole(take).then(() => api.windowRole().then(take));
} else draw(asked);
