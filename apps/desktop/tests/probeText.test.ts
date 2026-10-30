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
    // With no size, it is not said twice.
    const unsure = { size: null, resumable: false, ranges: true };
    expect(probeSentence(t("ar"), "ar", "latn", unsure)).toBe(
      "الحجم غير معروف. السيرفر يدعم التقسيم، لكنه لا يعطي ما يُتأكَّد به أن الملف لم يتغيّر، فسيُحمَّل باتصال واحد، ويبدأ من جديد إذا انقطع.",
    );
    expect(probeSentence(t("en"), "en", "latn", unsure)).toBe(
      "Size unknown. The server allows ranges but gives no way to be sure the file has not changed, so it downloads over one connection and starts over if cut.",
    );
    expect(probeSentence(t("ar"), "ar", "latn", { ...unsure, size: 10 })).toContain("لا يعطي الحجم");
    for (const lang of LANGS) expect(probeSentence(t(lang), lang, "latn", unsure)).not.toContain("?");
  });

  it("gives a known size as before", () => {
    const known = { size: 2458, resumable: true, ranges: true };
    expect(strip(sizeText(t("en"), "en", "latn", known))).toBe("2 KiB");
    expect(probeSentence(t("en"), "en", "latn", known)).toMatch(/\. The server allows ranges/);
    expect(probeSentence(t("ar"), "ar", "latn", known)).not.toContain("غير معروف");
  });

  it("warns, in both languages, when the link is a web page", () => {
    for (const lang of LANGS) expect(makeBundle(lang).hasMessage("add-web-page"), lang).toBe(true);
    expect(t("ar")("add-web-page")).toBe(
      "هذا الرابط صفحة ويب، لا ملف. إن كان فيها فيديو، فاضغط «حمّل الفيديو من الصفحة». وإلا فافتح الصفحة في المتصفح واضغط زر التحميل فيها.",
    );
    expect(t("en")("add-web-page")).toMatch(/^This link is a web page, not a file\./);
  });
});
