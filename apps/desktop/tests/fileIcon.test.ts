import { describe, expect, it } from "vitest";
import { nameIcon, typeIcon } from "../src/lib/fileIcon";

describe("the coloured file icons", () => {
  it("follow the category, and a plain page for none or a user's own", () => {
    expect(typeIcon("archives")).toBe("t-archives");
    expect(typeIcon("video")).toBe("t-video");
    expect(typeIcon(null)).toBe("t-file");
    expect(typeIcon("my-category")).toBe("t-file");
  });

  it("follow the extension, in any case, where no category is known", () => {
    expect(nameIcon("تقرير المبيعات.PDF")).toBe("t-documents");
    expect(nameIcon("ubuntu.iso.zip")).toBe("t-archives");
    expect(nameIcon("setup.EXE")).toBe("t-programs");
    expect(nameIcon("photo.heic")).toBe("t-images");
    expect(nameIcon("README")).toBe("t-file");
    expect(nameIcon(".mp3")).toBe("t-music");
  });
});
