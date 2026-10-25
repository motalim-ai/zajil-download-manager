import { describe, expect, it } from "vitest";
import { format, LANGS } from "../src/lib/i18n.svelte";
import { asRefreshProblem, refreshText } from "../src/lib/refresh";
import type { RefreshProblem } from "../src/lib/bindings/RefreshProblem";

const strip = (s: string) => s.replace(/[\u2068\u2069]/g, "");

// Every refusal of a new link (M5) is said in both languages, never as a
// raw key, and the sizes of another file are shown.
describe("refreshing the download link", () => {
  const problems: RefreshProblem[] = [
    { kind: "not_stopped" },
    { kind: "stopping" },
    { kind: "busy" },
    { kind: "cancelled" },
    { kind: "done" },
    { kind: "bad_link", reason: "a URL with a password" },
    { kind: "failed", error: "the server answered 403 Forbidden" },
    { kind: "not_the_same_file", why: "size", theirs: 1000, ours: 2048 },
    { kind: "not_the_same_file", why: "size", theirs: null, ours: 2048 },
    { kind: "not_the_same_file", why: "version", theirs: null, ours: null },
    { kind: "not_the_same_file", why: "validator_kind", theirs: null, ours: null },
    { kind: "not_the_same_file", why: "new_no_validator", theirs: null, ours: null },
    { kind: "not_the_same_file", why: "no_validator", theirs: null, ours: null },
  ];

  for (const lang of LANGS) {
    it(`says every problem in ${lang}`, () => {
      const t = (key: string, args?: Record<string, string>) => format(lang, key, args);
      const texts = problems.map((p) => refreshText(t, lang, "latn", p));
      for (const { text } of texts) expect(text).not.toMatch(/^refresh-|^details-/);
      expect(new Set(texts.map((s) => s.text)).size).toBe(texts.length);
      const at = (kind: string, why?: string, known = true) =>
        texts[problems.findIndex((p) => p.kind === kind && (!why || ("why" in p && p.why === why && (p.theirs !== null) === known)))]!;
      expect(strip(at("not_the_same_file", "size").text)).toContain("1,000");
      expect(at("not_the_same_file", "size", false).text).toBe(format(lang, "refresh-size-unknown"));
      expect(at("failed").detail).toBe("the server answered 403 Forbidden");
    });
  }

  it("reads the program's refusal, and anything else as a failure", () => {
    expect(asRefreshProblem({ kind: "done" })).toEqual({ kind: "done" });
    expect(asRefreshProblem("no connection")).toEqual({ kind: "failed", error: "no connection" });
    expect(asRefreshProblem(null)).toEqual({ kind: "failed", error: "null" });
  });
});
