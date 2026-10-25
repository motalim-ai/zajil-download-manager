import { mount } from "svelte";
import "./styles/app.css";
import App from "./App.svelte";
import LinkWindow from "./windows/LinkWindow.svelte";
import InfoWindow from "./windows/InfoWindow.svelte";

// One page for every window (M5.5): `?win=` says which one this is.
const target = document.getElementById("app");
if (!target) throw new Error("no #app element");
const win = new URLSearchParams(location.search).get("win");
const Root = win === "add-link" ? LinkWindow : win === "add-info" ? InfoWindow : App;
export default mount(Root, { target });
