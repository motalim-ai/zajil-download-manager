import { describe, expect, it } from "vitest";
import { Unseen } from "../src/lib/unseen";

describe("the list while the window is hidden", () => {
  it("is not drawn, and waits for the window to show", () => {
    const unseen = new Unseen<number>();
    expect(unseen.take(1, false)).toBe(1);
    expect(unseen.waiting).toBe(false);
    expect(unseen.take(2, true)).toBeNull();
    expect(unseen.take(3, true)).toBeNull();
    expect(unseen.waiting).toBe(true);
    // Shown: the next read is drawn, and nothing waits any more.
    expect(unseen.take(4, false)).toBe(4);
    expect(unseen.waiting).toBe(false);
  });

  it("reads the list again for a progress tick missed while hidden", () => {
    const unseen = new Unseen<number>();
    unseen.miss();
    expect(unseen.waiting).toBe(true);
    expect(unseen.take(5, false)).toBe(5);
    expect(unseen.waiting).toBe(false);
  });
});
