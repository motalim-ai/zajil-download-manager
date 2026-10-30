import { describe, expect, it } from "vitest";
import { newestFirst } from "../src/lib/order";

// The list's order (owner's request, 2026-10-30).
describe("the list's order", () => {
  it("is the newest first; one without a time after, the later of them first", () => {
    const list = [
      { id: 1, added: "2026-10-29T10:00:00Z" },
      { id: 2, added: null },
      { id: 3, added: "2026-10-30T09:00:00Z" },
      { id: 4, added: null },
      { id: 5, added: "2026-10-29T12:00:00Z" },
    ];
    expect(newestFirst(list).map((d) => d.id)).toEqual([3, 5, 1, 4, 2]);
  });
});
