import { describe, expect, it } from "vitest";
import { percent, size, speed, left } from "../src/lib/format";

const strip = (s: string) => s.replace(/[⁨⁩]/g, "");

describe("format", () => {
  it("sizes and speeds in Arabic, with Western or Eastern digits", () => {
    expect(strip(size("ar", "latn", 6.1 * 1024 ** 3))).toBe("6.1 جيجابايت");
    expect(strip(size("ar", "arab", 512 * 1024 ** 2))).toBe("٥١٢ ميجابايت");
    expect(strip(speed("en", "latn", 4.8 * 1024 ** 2))).toBe("4.8 MiB/s");
    expect(speed("ar", "latn", 0)).toBe("");
  });
  it("chooses the unit after rounding", () => {
    expect(strip(size("en", "latn", 1024 * 1024 - 1))).toBe("1 MiB");
    expect(strip(size("en", "latn", 1024 ** 3 - 1))).toBe("1 GiB");
    expect(strip(size("en", "latn", 1023 * 1024))).toBe("1,023 KiB");
  });
  it("never shows 100 % before the end", () => {
    expect(percent("en", "latn", 0.999)).toBe("99%");
    expect(percent("en", "latn", 1)).toBe("100%");
  });
  it("what is left, in its plural form", () => {
    expect(strip(left("ar", "latn", 9 * 60 * 1024, 1024))).toBe("9 دقائق");
    expect(strip(left("ar", "latn", 2 * 1024, 1024))).toBe("ثانيتان");
  });
});
