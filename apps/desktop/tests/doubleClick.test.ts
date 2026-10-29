import { beforeEach, describe, expect, it } from "vitest";
import { loadDoubleClick, saveDoubleClick } from "../src/lib/doubleClick";

// The page's storage, in memory (the tests run without a browser).
const kept = new Map<string, string>();
Object.defineProperty(globalThis, "localStorage", {
  configurable: true,
  value: {
    getItem: (k: string) => kept.get(k) ?? null,
    setItem: (k: string, v: string) => void kept.set(k, v),
    clear: () => kept.clear(),
  },
});

describe("the double-click setting", () => {
  beforeEach(() => kept.clear());

  it("opens the file at first, and keeps the user's choice", () => {
    expect(loadDoubleClick()).toBe("open");
    saveDoubleClick("folder");
    expect(loadDoubleClick()).toBe("folder");
    saveDoubleClick("details");
    expect(loadDoubleClick()).toBe("details");
  });

  it("reads anything else as the default", () => {
    kept.set("zajil.double-click", "rm -rf");
    expect(loadDoubleClick()).toBe("open");
  });
});
