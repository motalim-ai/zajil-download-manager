import { describe, expect, it } from "vitest";
import { DEFAULT, clean, load, save, shift, shown, toggle } from "../src/lib/toolbar";

// The toolbar as the user leaves it (M5.5).
const memory = () => {
  const kept = new Map<string, string>();
  return { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
};

describe("the toolbar", () => {
  it("is the design's at first, with its lines between groups", () => {
    const list = shown(DEFAULT);
    expect(list.map((b) => b.action)).toEqual(DEFAULT.order);
    expect(list.filter((b) => b.line).map((b) => b.action)).toEqual(["resume", "delete", "settings", "run-queue"]);
  });

  it("comes back as it was left: small, reordered, some hidden", () => {
    const storage = memory();
    const changed = { ...toggle(shift(DEFAULT, "pause", -1), "schedule"), small: true };
    save(changed, storage);
    expect(load(storage)).toEqual(changed);
    expect(shown(load(storage)).map((b) => b.action).slice(0, 3)).toEqual(["add", "pause", "resume"]);
    expect(shown(load(storage)).some((b) => b.action === "schedule")).toBe(false);
  });

  it("drops what does not belong, and is the default when unreadable", () => {
    expect(clean({ order: ["nope", "add", "add"], hidden: ["x", "pause"], small: "yes" })).toEqual({
      order: DEFAULT.order,
      hidden: ["pause"],
      small: false,
    });
    expect(load({ getItem: () => "{bad" })).toEqual(DEFAULT);
    expect(shift(DEFAULT, "add", -1)).toBe(DEFAULT);
    expect(shift(DEFAULT, "halt-queue", 1)).toBe(DEFAULT);
  });

  it("puts a new button after the one before it, in a toolbar kept from before", () => {
    const before = DEFAULT.order.filter((a) => a !== "resume-all");
    const kept = clean({ order: before, hidden: [], small: false });
    expect(kept.order.indexOf("resume-all")).toBe(kept.order.indexOf("pause-all") + 1);
  });
});
