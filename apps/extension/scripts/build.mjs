// Builds the extension for each browser family into dist/: dist/chrome (Chrome
// and Edge) and dist/firefox. The same code; only the manifest differs. The
// icons are the app's own, copied as they are (assets/icon, `tauri icon`).
import { cpSync, mkdirSync, rmSync, copyFileSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";

const root = join(dirname(fileURLToPath(import.meta.url)), "..");
const icons = join(root, "..", "desktop", "src-tauri", "icons");
const files = [
  "background.js",
  "capture.js",
  "compat.js",
  "cookies.js",
  "link.js",
  "page.js",
  "settings.js",
  "streams.js",
  "video-button.js",
  "yt-formats.js",
  "yt-page.js",
  "click-keys.js",
  "options.html",
  "options.js",
  "options.css",
  "welcome.html",
  "welcome.js",
];

for (const browser of ["chrome", "firefox"]) {
  const out = join(root, "dist", browser);
  rmSync(out, { recursive: true, force: true });
  mkdirSync(join(out, "icons"), { recursive: true });
  for (const file of files) copyFileSync(join(root, "src", file), join(out, file));
  cpSync(join(root, "src", "_locales"), join(out, "_locales"), { recursive: true });
  copyFileSync(join(root, "src", `manifest.${browser}.json`), join(out, "manifest.json"));
  for (const size of [32, 64, 128]) copyFileSync(join(icons, `${size}x${size}.png`), join(out, "icons", `${size}.png`));
  console.log(`built ${out}`);
}
