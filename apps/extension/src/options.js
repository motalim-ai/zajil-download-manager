// @ts-check
// The settings page: capture on or off, the file types, the excluded sites,
// the keys of a click.

import { api } from "./compat.js";
import { askForSites, dropSites, permissionView } from "./cookies.js";
import { localize } from "./page.js";
import { KEY_CHOICES, clickKeys, defaultKeys, defaults, isMac, load, save } from "./settings.js";

/** @param {string} id */
const $ = (id) => /** @type {any} */ (document.getElementById(id));

localize(api, document);

/** "Allow Zajil on sites": what it is now, and the buttons that change it. */
async function showCookies() {
  const view = await permissionView(api);
  $("cookies-state").textContent = api.i18n.getMessage(view.text);
  $("allow").hidden = !view.allow;
  $("drop").hidden = !view.drop;
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

const mac = isMac(/** @type {any} */ (navigator));

/** Each choice of keys, named as on this system's keyboard: Option on a
 * Mac, where Control+click is the right click, so not offered.
 * @param {string} id */
function fillKeys(id) {
  const names = mac ? { ctrl: "Control", alt: "Option", shift: "Shift" } : { ctrl: "Ctrl", alt: "Alt", shift: "Shift" };
  for (const choice of KEY_CHOICES.filter((c) => !(mac && c.includes("ctrl")))) {
    const option = document.createElement("option");
    option.value = choice;
    option.textContent =
      choice === "off"
        ? api.i18n.getMessage("keysOff")
        : [...choice.split("+").map((k) => names[/** @type {keyof typeof names} */ (k)]), api.i18n.getMessage("keysClick")].join("+");
    $(id).append(option);
  }
}
fillKeys("send-keys");
fillKeys("browser-keys");

/** The keys are chosen only once they are turned on. */
function showKeys() {
  const off = !$("use-keys").checked;
  $("send-keys").disabled = off;
  $("browser-keys").disabled = off;
}
$("use-keys").addEventListener("change", showKeys);

/** @param {import("./settings.js").Settings} settings */
function show(settings) {
  $("use-keys").checked = settings.useKeys;
  showKeys();
  const keys = clickKeys(settings, mac);
  $("send-keys").value = keys.send;
  $("browser-keys").value = keys.browser;
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
    useKeys: $("use-keys").checked,
    // This system's default is kept as "the default" (null), so a later
    // change of the defaults still reaches it (review of #169).
    sendKeys: $("send-keys").value === defaultKeys(mac).send ? null : $("send-keys").value,
    browserKeys: $("browser-keys").value === defaultKeys(mac).browser ? null : $("browser-keys").value,
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
