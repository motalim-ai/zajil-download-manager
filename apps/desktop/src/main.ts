import { mount, type Component } from "svelte";
import "./styles/app.css";
import { completeApi, loadShow } from "./lib/completeApi";
import { api } from "./lib/api";
import { loadSettings, startWindow } from "./lib/windowStart";
import { play, type Sound } from "./lib/sounds";
import type { Role } from "./lib/bindings/Role";

// One page for every window (M5.5): `?win=` says which one this is.
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const asked = new URLSearchParams(location.search).get("win");

// Each window loads its own code only (the owner's choice, after
// measuring: every window, the spare one too, loaded all of them).
type Page = () => Promise<{ default: Component<any> }>;
const pages: Record<string, Page> = {
  "add-link": () => import("./windows/LinkWindow.svelte"),
  "add-info": () => import("./windows/InfoWindow.svelte"),
  schedule: () => import("./windows/ScheduleWindow.svelte"),
  complete: () => import("./windows/CompleteWindow.svelte"),
  drop: () => import("./windows/DropBox.svelte"),
  video: () => import("./windows/VideoWindow.svelte"),
  batch: () => import("./windows/BatchWindow.svelte"),
  grab: () => import("./windows/GrabWindow.svelte"),
  settings: () => import("./windows/SettingsWindow.svelte"),
};

/** Draws window `win` (`null`: the main one). */
async function draw(win: string | null) {
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
  if (progress) {
    const { default: ProgressWindow } = await import("./windows/ProgressWindow.svelte");
    mount(ProgressWindow, { target: target!, props: { id: Number(progress[1]) } });
  } else {
    const page = (win !== null && pages[win]) || (() => import("./App.svelte"));
    const { default: Window } = await page();
    mount(Window, { target: target! });
  }
  // Drawn (timed with `ZAJIL_MEASURE=1`): at once, not at the next frame,
  // which a window behind another app may never get. The main page says
  // it itself once its list is drawn and it listens (`App.svelte`): what
  // waited for it then goes (`main_window.rs`).
  if (win !== null) void api.windowPainted().catch(() => {});
}

// A window made beforehand, hidden (`windows.rs`): its page loads and its
// settings are read now; it is drawn when it takes a role and shows.
if (asked === "sound") {
  // A sound while the main window is freed in the tray (`sounds.rs`): only
  // the tones, in a window never shown, which Zajil closes.
  const sound = new URLSearchParams(location.search).get("sound");
  if (sound === "done" || sound === "failed" || sound === "queue") play(sound as Sound);
} else if (asked === "spare") {
  void startWindow();
  // The one spare is «Download file info» (`windows.rs`): its code loads
  // now, not once it takes its role.
  void pages["add-info"]?.().catch(() => {});
  let drawn = false;
  const take = (role: Role | null) => {
    if (drawn || !role) return;
    drawn = true;
    void loadSettings();
    void draw(role.page);
  };
  void api.onWindowRole(take).then(() => api.windowRole().then(take));
} else void draw(asked);
