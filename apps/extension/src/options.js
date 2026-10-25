// @ts-check
// The settings page: capture on or off, the file types, the excluded sites.

import { api } from "./compat.js";
import { defaults, load, save } from "./settings.js";

/** @param {string} id */
const $ = (id) => /** @type {any} */ (document.getElementById(id));

const ui = api.i18n.getUILanguage?.() ?? "ar";
document.documentElement.lang = ui.startsWith("ar") ? "ar" : "en";
document.documentElement.dir = ui.startsWith("ar") ? "rtl" : "ltr";
for (const node of document.querySelectorAll("[data-i18n]")) {
  node.textContent = api.i18n.getMessage(/** @type {HTMLElement} */ (node).dataset.i18n ?? "") || node.textContent;
}

/** @param {import("./settings.js").Settings} settings */
function show(settings) {
  $("capture").checked = settings.capture;
  $("extensions").value = settings.extensions.join(" ");
  $("sites").value = settings.excludedSites.join("\n");
}

/** @param {string} text */
const words = (text) => text.split(/[\s,]+/).filter(Boolean);

$("settings").addEventListener("submit", async (/** @type {Event} */ event) => {
  event.preventDefault();
  await save(api, {
    capture: $("capture").checked,
    extensions: words($("extensions").value),
    excludedSites: words($("sites").value),
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
