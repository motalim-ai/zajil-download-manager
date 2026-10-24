import { describe, expect, it } from "vitest";
import ar from "../../../locales/ar/zajil.ftl?raw";
import en from "../../../locales/en/zajil.ftl?raw";

// Every key the interface asks for exists in Arabic and in English (review
// of #57: 14 keys were missing from both files, and showed as raw keys; the
// Rust test only compares the two files with each other). Keys built at run
// time (`category-${id}`, `settings-theme-${value}`…) are checked by their
// own tests or by the screenshots.
const sources = import.meta.glob(["../src/**/*.svelte", "../src/**/*.ts", "!../src/lib/bindings/**"], {
  query: "?raw",
  import: "default",
  eager: true,
}) as Record<string, string>;

const keysOf = (ftl: string) =>
  new Set(
    ftl
      .split("\n")
      .map((line: string) => /^([a-z][a-z0-9-]*)\s*=/.exec(line)?.[1])
      .filter((key: string | undefined): key is string => !!key),
  );

describe("translation keys", () => {
  const used = new Map<string, string>();
  for (const [file, text] of Object.entries(sources)) {
    for (const m of text.matchAll(/\bt\(\s*"([a-z][a-z0-9-]*)"/g)) used.set(m[1]!, file);
  }

  it("finds the keys the interface uses", () => {
    expect(used.size).toBeGreaterThan(50);
  });

  for (const [lang, ftl] of [["ar", ar], ["en", en]] as const) {
    it(`every key used exists in ${lang}`, () => {
      const have = keysOf(ftl);
      const missing = [...used].filter(([key]) => !have.has(key)).map(([key, file]) => `${key} (${file})`);
      expect(missing).toEqual([]);
    });
  }
});
