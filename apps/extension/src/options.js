// @ts-check
// The settings page: capture on or off, the file types, the excluded sites.

import { api } from "./compat.js";
import { askForSites, dropSites, sitesAllowed } from "./cookies.js";
import { localize } from "./page.js";
import { defaults, load, save } from "./settings.js";

/** @param {string} id */
const $ = (id) => /** @type {any} */ (document.getElementById(id));

localize(api, document);

/** "Allow Zajil on sites": what it is now, and the button that changes it. */
async function showCookies() {
  const allowed = await sitesAllowed(api);
  $("cookies-state").textContent = api.i18n.getMessage(allowed ? "cookiesAllowed" : "cookiesNotAllowed");
  $("allow").hidden = allowed;
  $("drop").hidden = !allowed;
}
$("allow").addEventListener("click", async () => {
  await askForSites(api);
  await showCookies();
});
$("drop").addEventListener("click", async () => {
  await dropSites(api);
  await showCookies();
});
await showCookies();

/** @param {import("./settings.js").Settings} settings */
function show(settings) {
  $("capture").checked = settings.capture;
  $("extensions").value = settings.extensions.join(" ");
  $("sites").value = settings.excludedSites.join("\n");
}

/** Types: separated by spaces or commas. @param {string} text */
const words = (text) => text.split(/[\s,]+/).filter(Boolean);
/** Sites: one per line (or separated by commas). @param {string} text */
const lines = (text) => text.split(/[\n,]+/).map((x) => x.trim()).filter(Boolean);

$("settings").addEventListener("submit", async (/** @type {Event} */ event) => {
  event.preventDefault();
  await save(api, {
    capture: $("capture").checked,
    extensions: words($("extensions").value),
    excludedSites: lines($("sites").value),
  });
  show(await load(api));
  $("saved").textContent = api.i18n.getMessage("optionsSaved");
});

$("reset").addEventListener("click", async () => {
  await save(api, defaults());
  show(await load(api));
  $("saved").textContent = api.i18n.getMessage("optionsSaved");
});

show(await load(api));
