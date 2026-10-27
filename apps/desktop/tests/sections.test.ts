import { describe, expect, it } from "vitest";
import { inSection } from "../src/lib/sections";

// «Unfinished», «Finished» and «Waiting» in the tree.
describe("the tree's sections", () => {
  it("put each status where IDM does", () => {
    const statuses = ["active", "waiting", "paused", "failed", "done"];
    expect(statuses.filter((s) => inSection("unfinished", s))).toEqual(["active", "waiting", "paused", "failed"]);
    expect(statuses.filter((s) => inSection("finished", s))).toEqual(["done"]);
    expect(statuses.filter((s) => inSection("waiting", s))).toEqual(["waiting"]);
  });
});
