import { describe, expect, it } from "vitest";
import { isProblem, problemText } from "../src/lib/batchText";
import { format, LANGS } from "../src/lib/i18n.svelte";

// «Batch download» (M7): every refusal Rust can give has its words in both
// languages, and the place in the link when it is known.
const KINDS = [
  "not_http", "too_long", "hidden", "no_host", "in_host", "in_fragment", "backslash", "unclosed", "unopened", "nested", "empty",
  "bad_range", "mixed", "descending", "zero_step", "width", "too_large", "bad_counter", "star_needs_range", "second_star", "no_star",
  "no_pattern", "too_many", "bad_url",
];

describe("a refused pattern", () => {
  for (const lang of LANGS) {
    const say = (key: string, args?: Record<string, string | number>) => format(lang, key, args);
    it(`says why in ${lang}, for every kind`, () => {
      for (const kind of KINDS) {
        const [what] = problemText(say, { kind, at: null, text: null, count: null, max: 1000 });
        expect(what, kind).not.toBe(`batch-error-${kind}`);
        expect(what, kind).not.toBe("");
      }
    });
    it(`says where, in ${lang}`, () => {
      const lines = problemText(say, { kind: "unclosed", at: 18, text: "[", count: null, max: null });
      expect(lines).toHaveLength(2);
      expect(lines[1]).toContain("18");
      expect(problemText(say, { kind: "too_many", at: null, text: null, count: 5000, max: 1000 })[0]).toMatch(/1[,٬]?000/);
    });
  }
  it("keeps the schemes in their own isolates", () => {
    const [what] = problemText((key, args) => format("ar", key, args), { kind: "not_http", at: null, text: null, count: null, max: null });
    expect(what).toContain("\u2068http://\u2069");
    expect(what).toContain("\u2068https://\u2069");
  });
  it("is told apart from a failure", () => {
    expect(isProblem({ kind: "mixed", at: 3, text: "1-b", count: null, max: null })).toBe(true);
    expect(isProblem("the window is gone")).toBe(false);
  });
});
