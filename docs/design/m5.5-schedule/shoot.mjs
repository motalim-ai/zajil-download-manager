// Pictures of the schedule window's design: its two tabs × Arabic and
// English × light and dark (8 pictures), as for the M5.5 design (#83).
//
// Usage, from the repository root, after `npm ci` in apps/desktop:
//   node docs/design/m5.5-schedule/shoot.mjs [filter]
// It needs Playwright (`playwright` or `playwright-core`) where Node finds
// it, and a Chromium it has downloaded; `CHROMIUM=<path>` names another.
// The font comes from the app's own @fontsource package: no network.
import fs from "node:fs";
import { createRequire } from "node:module";
const require = createRequire(import.meta.url);
const load = (name) => {
  try {
    return require(name);
  } catch {
    return null;
  }
};
const { chromium } = load("playwright") ?? load("playwright-core");
const only = process.argv[2];
const here = new URL(".", import.meta.url).pathname;
const html = `${here}index.html`;
const outDir = `${here}shots`;
const F = `${here}../../../apps/desktop/node_modules/@fontsource/ibm-plex-sans-arabic/files/ibm-plex-sans-arabic-`;
let css = "";
for (const sub of ["arabic", "latin"]) for (const w of [400, 500, 600, 700]) {
  const b64 = fs.readFileSync(`${F}${sub}-${w}-normal.woff2`).toString("base64");
  css += `@font-face{font-family:"IBM Plex Sans Arabic";font-weight:${w};font-style:normal;src:url(data:font/woff2;base64,${b64}) format("woff2");${sub === "arabic" ? "unicode-range:U+0600-06FF,U+0750-077F,U+0870-088E,U+0890-0891,U+0897-08E1,U+08E3-08FF,U+200C-200E,U+2010-2011,U+204F,U+2E41,U+FB50-FDFF,U+FE70-FE74,U+FE76-FEFC;" : ""}}`;
}
fs.mkdirSync(outDir, { recursive: true });
const browser = await chromium.launch(process.env.CHROMIUM ? { executablePath: process.env.CHROMIUM } : {});
const page = await browser.newPage({ deviceScaleFactor: 2, viewport: { width: 1400, height: 1000 } });
await page.route("https://fonts.googleapis.com/**", (r) => r.fulfill({ status: 200, contentType: "text/css", body: css }));
await page.route("https://fonts.gstatic.com/**", (r) => r.abort());
for (const lang of ["ar", "en"]) for (const theme of ["light", "dark"]) for (const w of ["schedule", "files"]) {
  const name = `${w}-${lang}-${theme}`;
  if (only && !name.includes(only)) continue;
  await page.goto(`file://${html}?lang=${lang}&theme=${theme}&win=${w}`);
  await page.evaluate(async () => { for (const w of [400, 600, 700]) await document.fonts.load(`${w} 14px "IBM Plex Sans Arabic"`, "زاجل Zajil"); await document.fonts.ready; });
  const ok = await page.evaluate(() => document.fonts.check('600 14px "IBM Plex Sans Arabic"', "زاجل"));
  if (!ok) throw new Error("font not loaded");
  await page.locator("#shot").screenshot({ path: `${outDir}/${name}.png` });
}
await browser.close();
