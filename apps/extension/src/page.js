// @ts-check
// What the extension's pages share: their texts and their language.

/**
 * Fills `[data-i18n]` with its text, and sets the page's language and
 * direction from the texts the browser chose (Arabic for any language but
 * English, as `default_locale`), not from the browser's own language.
 *
 * @param {any} browserApi
 * @param {Document} doc
 */
export function localize(browserApi, doc) {
  const lang = browserApi.i18n.getMessage("lang") || "ar";
  doc.documentElement.lang = lang;
  doc.documentElement.dir = lang === "ar" ? "rtl" : "ltr";
  for (const node of doc.querySelectorAll("[data-i18n]")) {
    const key = /** @type {HTMLElement} */ (node).dataset.i18n ?? "";
    node.textContent = browserApi.i18n.getMessage(key) || node.textContent;
  }
}
