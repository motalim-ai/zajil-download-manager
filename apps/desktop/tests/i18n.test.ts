import { describe, expect, it } from "vitest";
import { format, makeBundle, parseCount, pluralCategory, LANGS } from "../src/lib/i18n.svelte";

const strip = (s: string) => s.replace(/[⁨⁩]/g, "");

// CLDR's Arabic cardinal rule, written out (the same as the Rust test).
function cldrArabic(n: number): string {
  if (n === 0) return "zero";
  if (n === 1) return "one";
  if (n === 2) return "two";
  const m = n % 100;
  if (m >= 3 && m <= 10) return "few";
  if (m >= 11 && m <= 99) return "many";
  return "other";
}

describe("translations in the interface", () => {
  it("load without errors in both languages", () => {
    for (const lang of LANGS) expect(() => makeBundle(lang)).not.toThrow();
  });

  it("PLURAL follows CLDR for every number up to 1200, as in Rust", () => {
    for (let n = 0; n <= 1200; n++) {
      expect(pluralCategory("ar", n)).toBe(cldrArabic(n));
      expect(pluralCategory("en", n)).toBe(n === 1 ? "one" : "other");
    }
  });

  it("gives the six Arabic forms of 0, 1, 2, 3, 11 and 100, in both digit styles", () => {
    const latn = [0, 1, 2, 3, 11, 100].map((n) => strip(format("ar", "downloads-count", { count: n }, "latn")));
    expect(latn).toEqual(["لا تحميلات", "تحميل واحد", "تحميلان", "3 تحميلات", "11 تحميلًا", "100 تحميل"]);
    const arab = [3, 11, 103, 111].map((n) => strip(format("ar", "downloads-count", { count: n }, "arab")));
    expect(arab).toEqual(["٣ تحميلات", "١١ تحميلًا", "١٠٣ تحميلات", "١١١ تحميلًا"]);
  });

  it("reads counts given as text as Rust does, and refuses what is not a count", () => {
    expect(strip(format("ar", "downloads-count", { count: "١١" }))).toBe("١١ تحميلًا");
    expect(strip(format("ar", "downloads-count", { count: "1,003" }))).toBe("1,003 تحميلات");
    expect(strip(format("ar", "downloads-count", { count: "١٬٠٠٣" }))).toBe("١٬٠٠٣ تحميلات");
    expect(strip(format("ar", "downloads-count", { count: "٣٫٥" }))).toBe("٣٫٥ تحميل");
    expect(parseCount(" \u00A03\t")).toBe(3);
    for (const bad of ["", "abc", "3 files", "1,5", "1e3", "inf", "NaN", "+3", "-3", "1,00", "١٬٠٠", "\uFEFF3"]) {
      expect(parseCount(bad), bad).toBeNull();
      expect(format("ar", "downloads-count", { count: bad }), bad).toBe("downloads-count");
    }
    for (const bad of [-1, Infinity, NaN]) expect(format("ar", "downloads-count", { count: bad })).toBe("downloads-count");
    expect(strip(format("en", "downloads-count", { count: -0 }))).toBe("0 downloads");
  });

  it("keeps a value inside its isolation, and its words apart", () => {
    const shown = format("ar", "notify-failed-body", { name: "a.mp4", reason: "x\u2069\u202Egpj.exe" });
    expect(shown).toBe("\u2068a.mp4\u2069: \u2068xgpj.exe\u2069");
    expect(strip(format("en", "notify-failed-body", { name: "a", reason: "timed out\nretrying" }))).toBe("a: timed out retrying");
  });

  it("shows a missing key as the key itself", () => {
    expect(format("ar", "no-such-key")).toBe("no-such-key");
  });
});
