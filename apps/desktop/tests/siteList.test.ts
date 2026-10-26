import { describe, expect, it } from "vitest";
import { checkList, normalizeSite } from "../src/lib/siteList";

describe("the per-site limits list (#165)", () => {
  it("makes a typed site the file's form", () => {
    expect(normalizeSite("Example.COM")).toBe("example.com");
    expect(normalizeSite(" https://dl.example.com/files/a.zip ")).toBe("dl.example.com");
    expect(normalizeSite("example.com:8080")).toBe("example.com");
    expect(normalizeSite("example.com.")).toBe("example.com");
    expect(normalizeSite("مثال.com")).toBe("xn--mgbh0fb.com");
    for (const bad of ["", "two words.com", "*.example.com", "http://", "."]) expect(normalizeSite(bad), bad).toBeNull();
  });

  it("checks the list as the settings file does", () => {
    expect(checkList([{ site: "Example.com", connections: 2 }])).toEqual({ list: [{ site: "example.com", connections: 2 }] });
    expect(checkList([{ site: "a.com", connections: 2 }, { site: "https://A.com/x", connections: 3 }])).toEqual({
      problem: { key: "sites-twice", site: "a.com" },
    });
    expect(checkList([{ site: "a.com", connections: 0 }])).toEqual({ problem: { key: "sites-bad-number", site: "a.com" } });
    expect(checkList([{ site: "a.com", connections: 33 }])).toEqual({ problem: { key: "sites-bad-number", site: "a.com" } });
    expect(checkList([{ site: "*.a.com", connections: 2 }])).toEqual({ problem: { key: "sites-bad-site", site: "*.a.com" } });
    expect(checkList([])).toEqual({ list: [] });
  });
});
