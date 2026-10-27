import { describe, expect, it } from "vitest";
import { categoryFolder, chosenFolder, savePath, typedPath } from "../src/lib/addFolder";
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

describe("«Save as»", () => {
  it("shows the folder and the name as one path, with the folder's separator", () => {
    expect(savePath("/d/Videos", "a.mp4")).toBe("/d/Videos/a.mp4");
    expect(savePath("/d/", "a.mp4")).toBe("/d/a.mp4");
    expect(savePath("C:\\Users\\me\\Downloads", "a.mp4")).toBe("C:\\Users\\me\\Downloads\\a.mp4");
    expect(savePath("", "a.mp4")).toBe("a.mp4");
  });
  it("reads a typed path as a folder and a name", () => {
    expect(typedPath("/d/Videos/b.mp4", "/d/Videos")).toEqual({ folder: null, name: "b.mp4" });
    expect(typedPath(" /e/Films/b.mp4 ", "/d/Videos")).toEqual({ folder: "/e/Films", name: "b.mp4" });
    expect(typedPath("C:\\Films\\b.mp4", "C:\\Users\\me")).toEqual({ folder: "C:\\Films", name: "b.mp4" });
    // A name alone keeps the folder; an emptied name means the server's.
    expect(typedPath("b.mp4", "/d")).toEqual({ folder: null, name: "b.mp4" });
    expect(typedPath("/d/Videos/", "/d/Videos")).toEqual({ folder: null, name: "" });
  });
});
