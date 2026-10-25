import { describe, expect, it } from "vitest";
import { lastTry } from "../src/lib/format";
import { format } from "../src/lib/i18n.svelte";

// «Last try» in the table (#95): the time today, «yesterday» and the time,
// else the date; in local time.
const strip = (s: string) => s.replace(/[⁨⁩]/g, "");
const t = (lang: "ar" | "en") => (key: string, args?: Record<string, string | number>) => strip(format(lang, key, args ?? {}));
const now = new Date(2026, 9, 25, 20, 0);
const at = (d: number, h: number, m: number) => new Date(2026, 9, d, h, m).toISOString();

describe("«Last try»", () => {
  it("is the time today", () => {
    expect(lastTry("en", "latn", at(25, 9, 5), t("en"), now)).toBe("09:05");
    expect(lastTry("ar", "arab", at(25, 9, 5), t("ar"), now)).toBe("٠٩:٠٥");
  });

  it("is «yesterday» and the time the day before", () => {
    expect(strip(lastTry("ar", "latn", at(24, 22, 40), t("ar"), now))).toBe("أمس 22:40");
    expect(strip(lastTry("en", "latn", at(24, 22, 40), t("en"), now))).toBe("Yesterday 22:40");
  });

  it("is the date before that, and nothing for a time it cannot read", () => {
    expect(lastTry("en", "latn", at(20, 8, 0), t("en"), now)).toMatch(/20/);
    expect(lastTry("en", "latn", "not a time", t("en"), now)).toBe("");
  });
});
