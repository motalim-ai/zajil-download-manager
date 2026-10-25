// @ts-check
// The welcome page, opened once at install: what the extension does, and
// "Allow Zajil on sites" (the cookies and the video features), asked for
// only when the user presses the button.

import { api } from "./compat.js";
import { localize } from "./page.js";
import { askForSites, sitesAllowed } from "./cookies.js";

localize(api, document);

/** @param {string} id */
const $ = (id) => /** @type {any} */ (document.getElementById(id));

async function show() {
  const allowed = await sitesAllowed(api);
  $("cookies-state").textContent = api.i18n.getMessage(allowed ? "cookiesAllowed" : "cookiesNotAllowed");
  $("allow").hidden = allowed;
}

$("allow").addEventListener("click", async () => {
  await askForSites(api);
  await show();
});

await show();
