import { mount } from "svelte";
import "./styles/app.css";
import App from "./App.svelte";
import LinkWindow from "./windows/LinkWindow.svelte";
import InfoWindow from "./windows/InfoWindow.svelte";
import ProgressWindow from "./windows/ProgressWindow.svelte";
import ScheduleWindow from "./windows/ScheduleWindow.svelte";
import CompleteWindow from "./windows/CompleteWindow.svelte";
import { completeApi, loadShow } from "./lib/completeApi";

// One page for every window (M5.5): `?win=` says which one this is.
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const win = new URLSearchParams(location.search).get("win");
// Rust learns at each start whether «Download complete» shows (the user's
// choice is kept by the page).
if (win === null) void completeApi.setShow(loadShow()).catch(() => {});
const progress = /^progress-(\d+)$/.exec(win ?? "");
export default progress
  ? mount(ProgressWindow, { target, props: { id: Number(progress[1]) } })
  : mount(
      win === "add-link" ? LinkWindow : win === "add-info" ? InfoWindow : win === "schedule" ? ScheduleWindow : win === "complete" ? CompleteWindow : App,
      { target },
    );
