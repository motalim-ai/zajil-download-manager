// The tray icon of macOS, made from the approved mark itself
// (assets/icon/zajil-mark.svg, one colour by design, "for the system
// tray"): drawn in black on transparency at 44 x 44 px (22 pt at 2x), the
// form macOS asks of a template image, which it then colours to match the
// menu bar. The mark file is not changed; only `currentColor` is given a
// value for this drawing. Windows and Linux use the app icon in colour.
//
//   node scripts/tray-icon.mjs      (writes src-tauri/icons/tray-template.png)
import { readFileSync, writeFileSync } from "node:fs";
import { Resvg } from "@resvg/resvg-js";

const svg = readFileSync(new URL("../../../assets/icon/zajil-mark.svg", import.meta.url), "utf8");
const png = new Resvg(svg.replaceAll("currentColor", "#000000"), {
  fitTo: { mode: "width", value: 44 },
  background: "rgba(0, 0, 0, 0)",
})
  .render()
  .asPng();
writeFileSync(new URL("../src-tauri/icons/tray-template.png", import.meta.url), png);
console.log(`src-tauri/icons/tray-template.png: ${png.length} bytes`);
