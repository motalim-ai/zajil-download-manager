import { describe, expect, it } from "vitest";
import { isolatedName, nameDir, nameParts } from "../src/lib/fname";

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
  // Review of #46: a word that starts with digits («1080p», «3D») was cut
  // in two, and drawn «p.mkv1080».
  it("keeps digits stuck to the Latin word with it", () => {
    expect(nameParts("فيلم 1080p.mkv")).toEqual({ head: "فيلم ", tail: "1080p.mkv" });
    expect(nameParts("تقرير 3D.pdf")).toEqual({ head: "تقرير ", tail: "3D.pdf" });
    expect(nameParts("درس 12-Part.mp4")).toEqual({ head: "درس ", tail: "12-Part.mp4" });
    expect(nameParts("ملف -v2.zip")).toEqual({ head: "ملف -", tail: "v2.zip" });
    expect(nameParts("حلقة (720p).mkv")).toEqual({ head: "حلقة ", tail: "(720p).mkv" });
  });
  it("orders Arabic-Indic digits as numbers, and knows Greek and Cyrillic", () => {
    expect(nameParts("محاضرة Part ٢.mp4")).toEqual({ head: "محاضرة ", tail: "Part ٢.mp4" });
    expect(nameParts("تقرير (Привет).pdf")).toEqual({ head: "تقرير ", tail: "(Привет).pdf" });
    expect(nameParts("ملف \uFEFFdoc.txt")).toEqual({ head: "ملف \uFEFF", tail: "doc.txt" });
  });
  it("always gives back the whole name", () => {
    for (const name of ["", ".", ".bashrc", "file.", "a..b", "x.tar.gz", "ملف.", "تقرير 3D", "\u202Eأ b.c", "😀 a.b"]) {
      const { head, tail } = nameParts(name);
      expect(head + tail, name).toBe(name);
    }
  });
});

describe("nameDir", () => {
  it("is the direction of the first strong letter, as a name box gives it", () => {
    expect(nameDir("محاضرة 12 - الخوارزميات (Part 2).mp4")).toBe("rtl");
    expect(nameDir("Blender 4.5 تعليم النمذجة.mkv")).toBe("ltr");
    expect(nameDir("2026 تقرير.pdf")).toBe("rtl");
    expect(nameDir("12345")).toBe("ltr");
  });
});

describe("isolatedName", () => {
  it("gives a window's title FileName's two isolates as marks", () => {
    expect(isolatedName("محاضرة 12 - الخوارزميات (Part 2).mp4")).toBe("⁨محاضرة 12 - الخوارزميات ⁦(Part 2).mp4⁩⁩");
    expect(isolatedName("report.pdf")).toBe("⁨report.pdf⁩");
  });
});
