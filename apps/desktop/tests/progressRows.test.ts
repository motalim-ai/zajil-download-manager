import { describe, expect, it } from "vitest";
import { patchRows } from "../src/lib/progressRows";

describe("a progress tick", () => {
  it("changes only the running rows, in place", () => {
    const list = [
      { id: 1, done: 0 },
      { id: 2, done: 5 },
      { id: 3, done: 9 },
    ];
    const untouched = list[1];
    expect(patchRows(list, [{ id: 3, done: 10 }, { id: 1, done: 4 }])).toBe(true);
    expect(list).toEqual([
      { id: 1, done: 4 },
      { id: 2, done: 5 },
      { id: 3, done: 10 },
    ]);
    expect(list[1]).toBe(untouched);
  });

  it("asks for the whole list for a row it does not have", () => {
    const list = [{ id: 1, done: 0 }];
    expect(patchRows(list, [{ id: 1, done: 3 }, { id: 7, done: 1 }])).toBe(false);
    expect(list).toEqual([{ id: 1, done: 0 }]);
  });
});
