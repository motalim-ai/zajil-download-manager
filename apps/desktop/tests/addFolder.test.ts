import { describe, expect, it } from "vitest";
import { categoryFolder, chosenFolder } from "../src/lib/addFolder";
import type { CategoryView } from "../src/lib/bindings/CategoryView";

// «Download file info»: the folder follows the picked category until the
// user picks a folder by hand (owner's request for 0.1).
const categories: CategoryView[] = [
  { id: "video", name: null, extensions: ["mp4"], sites: [], folder: "/d/Videos" },
  { id: "music", name: null, extensions: ["mp3"], sites: [], folder: null },
];
const base = { categories, detected: "video", detectedFolder: "/d/Videos", downloadFolder: "/d" };

describe("the info window's folder", () => {
  it("is the detected category's, and added by category, while nothing changes", () => {
    expect(chosenFolder({ ...base, picked: "video", byHand: null })).toEqual({ shown: "/d/Videos", request: null });
  });

  it("follows another category the user picks: its own folder, else the downloads folder", () => {
    expect(chosenFolder({ ...base, picked: "music", byHand: null })).toEqual({ shown: "/d", request: "/d" });
    expect(chosenFolder({ ...base, detected: "music", detectedFolder: "/d", picked: "video", byHand: null })).toEqual({
      shown: "/d/Videos",
      request: "/d/Videos",
    });
    expect(chosenFolder({ ...base, picked: null, byHand: null })).toEqual({ shown: "/d", request: "/d" });
  });

  it("stays the user's own once picked by hand, whatever the category", () => {
    expect(chosenFolder({ ...base, picked: "music", byHand: "/mine" })).toEqual({ shown: "/mine", request: "/mine" });
    expect(chosenFolder({ ...base, picked: "video", byHand: "/mine" })).toEqual({ shown: "/mine", request: "/mine" });
  });

  it("knows each category's folder", () => {
    expect(categoryFolder(categories, "video", "/d")).toBe("/d/Videos");
    expect(categoryFolder(categories, "music", "/d")).toBe("/d");
    expect(categoryFolder(categories, "gone", "/d")).toBe("/d");
  });
});
