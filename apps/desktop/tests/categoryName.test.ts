import { describe, expect, it } from "vitest";
import { categoryName } from "../src/lib/categoryName";

const t = (key: string) => `«${key}»`;

describe("a category's name on screen (#58)", () => {
  it("is the user's name when there is one", () => {
    expect(categoryName("programs", "برامجي", t)).toBe("برامجي");
    expect(categoryName("custom-1", "كتب", t)).toBe("كتب");
  });

  it("is a built-in category's translation, and a new one's id, never a translation key", () => {
    expect(categoryName("video", null, t)).toBe("«category-video»");
    expect(categoryName("custom-1", null, t)).toBe("custom-1");
    expect(categoryName("my-books", "", t)).toBe("my-books");
  });
});
