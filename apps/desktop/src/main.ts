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
import { completeApi, loadShow } from "./lib/completeApi";

// One page for every window (M5.5): `?win=` says which one this is.
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const win = new URLSearchParams(location.search).get("win");
// Rust learns at each start whether «Download complete» shows (the user's
// choice is kept by the page).
if (win === null) void completeApi.setShow(loadShow()).catch(() => {});
const progress = /^progress-(\d+)$/.exec(win ?? "");
// ui-test builds only: M5.5's end-to-end test drives each window
// (`src/uitest/e2e.ts`); a release build has none of it.
if (__UI_TEST__) void import("./uitest/e2e").then((test) => test.drive(win));
export default progress
  ? mount(ProgressWindow, { target, props: { id: Number(progress[1]) } })
  : mount(
      win === "add-link" ? LinkWindow : win === "add-info" ? InfoWindow : win === "schedule" ? ScheduleWindow : win === "complete" ? CompleteWindow : win === "drop" ? DropBox : win === "video" ? VideoWindow : win === "batch" ? BatchWindow : win === "grab" ? GrabWindow : App,
      { target },
    );
