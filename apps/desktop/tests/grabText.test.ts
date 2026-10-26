import { describe, expect, it } from "vitest";
import { LIMITS, limitText, listOf, refusalText } from "../src/lib/grabText";
import { format, LANGS } from "../src/lib/i18n.svelte";

// «Site grabber» (M7): every refusal and limit Rust can give has its words
// in both languages.
describe("the site grabber's texts", () => {
  it("reads the lists the user types", () => {
    expect(listOf(" zip, .PSD  rar،7z ")).toEqual(["zip", ".PSD", "rar", "7z"]);
    expect(listOf("  ")).toEqual([]);
  });

  for (const lang of LANGS) {
    const say = (key: string, args?: Record<string, string | number>) => format(lang, key, args);
    it(`says every refusal and limit in ${lang}`, () => {
      for (const error of ["start", "depth", "max_files", "max_total", "types", "site:a b"]) {
        const text = refusalText(say, error, { maxDepth: 10, maxFiles: 10000 });
        expect(text, error).not.toMatch(/^grab-error/);
        expect(text, error).not.toBe(error);
      }
      expect(refusalText(say, "site:a/b", { maxDepth: 10, maxFiles: 10000 })).toContain("a/b");
      for (const limit of LIMITS) expect(limitText(say, limit), limit).not.toMatch(/^grab-limit/);
      expect(refusalText(say, "the start page cannot be reached", { maxDepth: 10, maxFiles: 10000 })).toBe("the start page cannot be reached");
    });
    it(`keeps the schemes left to right in ${lang}`, () => {
      const text = refusalText(say, "start", { maxDepth: 10, maxFiles: 10000 });
      expect(text).toContain("⁨http://⁩");
    });
  }
});
