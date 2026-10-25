import { describe, expect, it } from "vitest";
import { format, makeBundle, LANGS, type Lang } from "../src/lib/i18n.svelte";
import { probeSentence, sizeText } from "../src/lib/probeText";

// «Download file info» after the owner's test with a Google Drive link:
// no «?.» when the server gives no size, and the web page warning.
const strip = (s: string) => s.replace(/[⁨⁩]/g, "");
const t = (lang: Lang) => (key: string, args?: Record<string, string | number>) => strip(format(lang, key, args ?? {}));

describe("«Download file info»", () => {
  it("says an unknown size in words, never «?»", () => {
    const one = { size: null, resumable: false, ranges: false };
    expect(sizeText(t("ar"), "ar", "latn", one)).toBe("الحجم غير معروف");
    expect(sizeText(t("en"), "en", "latn", one)).toBe("Size unknown");
    expect(probeSentence(t("ar"), "ar", "latn", one)).toBe("الحجم غير معروف. السيرفر لا يدعم التقسيم، فسيُحمَّل باتصال واحد.");
    expect(probeSentence(t("en"), "en", "latn", one)).toBe("Size unknown. The server does not allow ranges, so it downloads over one connection.");
    const unsure = { size: null, resumable: false, ranges: true };
    expect(probeSentence(t("ar"), "ar", "latn", unsure)).toMatch(/^الحجم غير معروف\. السيرفر يدعم التقسيم/);
    for (const lang of LANGS) expect(probeSentence(t(lang), lang, "latn", unsure)).not.toContain("?");
  });

  it("gives a known size as before", () => {
    const known = { size: 2458, resumable: true, ranges: true };
    expect(sizeText(t("en"), "en", "latn", known)).toMatch(/^2\.4 KB$|^2\.4 KiB$|KB|KiB/);
    expect(probeSentence(t("en"), "en", "latn", known)).toMatch(/\. The server allows ranges/);
    expect(probeSentence(t("ar"), "ar", "latn", known)).not.toContain("غير معروف");
  });

  it("warns, in both languages, when the link is a web page", () => {
    for (const lang of LANGS) expect(makeBundle(lang).hasMessage("add-web-page"), lang).toBe(true);
    expect(t("ar")("add-web-page")).toBe(
      "هذا الرابط صفحة ويب، لا ملف. لتحميل الملف افتح الصفحة في المتصفح واضغط زر التحميل فيها، أو استعمل رابط التحميل المباشر.",
    );
    expect(t("en")("add-web-page")).toMatch(/^This link is a web page, not a file\./);
  });
});
