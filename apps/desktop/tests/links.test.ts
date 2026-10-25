import { describe, expect, it } from "vitest";
import { firstLink } from "../src/lib/links";

// What the drop box takes from a drop (M5.5).
describe("the drop box's link", () => {
  it("is the first link of the browser's link list, comments left out", () => {
    expect(firstLink("# from the page\nhttps://a.example/x.zip\nhttps://b.example/y.iso", "")).toBe("https://a.example/x.zip");
  });
  it("is found in dropped text, without the punctuation around it", () => {
    expect(firstLink("", "حمّل من (https://a.example/x.zip)، لو سمحت.")).toBe("https://a.example/x.zip");
    expect(firstLink("", "see http://a.example/p?q=1&r=2.")).toBe("http://a.example/p?q=1&r=2");
  });
  it("is nothing when there is none, or only other kinds of links", () => {
    expect(firstLink("", "no link here")).toBeNull();
    expect(firstLink("file:///etc/passwd", "ftp://a.example/x")).toBeNull();
  });
});
