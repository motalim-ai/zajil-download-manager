import { describe, expect, it } from "vitest";
import { nameParts } from "../src/lib/fname";

// The rule of the approved design (owner's review): what goes in the
// inner left-to-right isolate.
describe("nameParts", () => {
  it("keeps a Latin ending and the extension together after an Arabic name", () => {
    expect(nameParts("محاضرة 12 - الخوارزميات (Part 2).mp4")).toEqual({
      head: "محاضرة 12 - الخوارزميات ",
      tail: "(Part 2).mp4",
    });
    expect(nameParts("أغاني فيروز - Live 1971.flac")).toEqual({ head: "أغاني فيروز - ", tail: "Live 1971.flac" });
  });
  it("leaves the extension in the flow after an Arabic word", () => {
    for (const name of ["تقرير المبيعات Q3-2026 النهائي.pdf", "صور الرحلة 2025 (الجزء 3).zip", "Blender 4.5 تعليم النمذجة.mkv"]) {
      expect(nameParts(name)).toEqual({ head: name, tail: "" });
    }
  });
  // The nine names of the demo list (the approved design), each with the
  // pieces the real component draws: the image test of the real window
  // (next PR) checks how they look; this checks the rule itself.
  it("splits each of the nine names of the design by the rule", () => {
    const cases: [string, string, string][] = [
      ["Ubuntu 24.04.3 desktop amd64.iso", "Ubuntu 24.04.3 desktop amd64.iso", ""],
      ["محاضرة 12 - الخوارزميات (Part 2).mp4", "محاضرة 12 - الخوارزميات ", "(Part 2).mp4"],
      ["تقرير المبيعات Q3-2026 النهائي.pdf", "تقرير المبيعات Q3-2026 النهائي.pdf", ""],
      ["أغاني فيروز - Live 1971.flac", "أغاني فيروز - ", "Live 1971.flac"],
      ["صور الرحلة 2025 (الجزء 3).zip", "صور الرحلة 2025 (الجزء 3).zip", ""],
      ["zajil-0.4.0-arm64.dmg", "zajil-0.4.0-arm64.dmg", ""],
      ["ديوان المتنبي (طبعة 2019).epub", "ديوان المتنبي (طبعة 2019).epub", ""],
      ["Blender 4.5 تعليم النمذجة.mkv", "Blender 4.5 تعليم النمذجة.mkv", ""],
      ["backup_2026-10-20 النسخة الكاملة.tar.gz", "backup_2026-10-20 النسخة الكاملة.tar.gz", ""],
    ];
    for (const [name, head, tail] of cases) {
      expect(nameParts(name), name).toEqual({ head, tail });
      // Nothing is lost or added: the pieces are the name.
      expect(head + tail).toBe(name);
    }
  });
  it("leaves Latin names and names without an extension alone", () => {
    expect(nameParts("Ubuntu 24.04.3 desktop amd64.iso")).toEqual({ head: "Ubuntu 24.04.3 desktop amd64.iso", tail: "" });
    expect(nameParts("README")).toEqual({ head: "README", tail: "" });
    expect(nameParts("ملف بلا امتداد")).toEqual({ head: "ملف بلا امتداد", tail: "" });
  });
});
