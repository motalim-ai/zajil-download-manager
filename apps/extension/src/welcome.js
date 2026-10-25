// @ts-check
// The welcome page, opened once at install: what the extension does, and
// the cookie permission, asked for only when the user presses the button.

import { api } from "./compat.js";
import { localize } from "./page.js";
import { askForCookies, cookiesAllowed } from "./cookies.js";

localize(api, document);

/** @param {string} id */
const $ = (id) => /** @type {any} */ (document.getElementById(id));

async function show() {
  const allowed = await cookiesAllowed(api);
  $("cookies-state").textContent = api.i18n.getMessage(allowed ? "cookiesAllowed" : "cookiesNotAllowed");
  $("allow").hidden = allowed;
}

$("allow").addEventListener("click", async () => {
  await askForCookies(api);
  await show();
});

await show();
