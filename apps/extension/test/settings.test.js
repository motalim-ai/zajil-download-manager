import { describe, expect, it } from "vitest";
import { DEFAULT_EXTENSIONS, defaults, load, normalizeExtensions, normalizeSites, save } from "../src/settings.js";
import { readFileSync } from "node:fs";

function fakeStorage(initial) {
  let data = initial === undefined ? {} : { settings: initial };
  return {
    storage: {
      local: {
        get: async () => structuredClone(data),
        set: async (value) => {
          data = structuredClone({ ...data, ...value });
        },
      },
    },
  };
}

describe("settings", () => {
  it("default to capturing Zajil's built-in categories, with no excluded site", async () => {
    expect(await load(fakeStorage())).toEqual(defaults());
    expect(defaults().capture).toBe(true);
  });

  it("the default types are exactly the built-in categories' extensions", () => {
    const rust = readFileSync(new URL("../../../crates/zajil-core/src/category.rs", import.meta.url), "utf8");
    const block = rust.slice(rust.indexOf("pub fn built_in()"), rust.indexOf("updated_at: now_rfc3339()", rust.indexOf("pub fn built_in()")));
    const fromRust = [...block.matchAll(/"([a-z0-9]+)"/g)].map((m) => m[1]).filter((x) => !["programs", "video", "music", "documents", "archives"].includes(x));
    expect(DEFAULT_EXTENSIONS).toEqual(fromRust);
  });

  it("are cleaned when saved and when read", async () => {
    const api = fakeStorage();
    await save(api, { capture: false, extensions: [".ZIP", " mp4 ", "", "a b", "zip", "x".repeat(17)], excludedSites: ["https://Example.com/path", "sub.test.", "not a site", "", "example.com"] });
    expect(await load(api)).toEqual({ capture: false, extensions: ["zip", "mp4"], excludedSites: ["example.com", "sub.test"] });
    expect(await load(fakeStorage({ capture: "yes", extensions: "zip", excludedSites: [42] }))).toEqual({ ...defaults(), excludedSites: [] });
  });

  it("normalize odd entries away", () => {
    expect(normalizeExtensions(["7Z", "tar.gz", null, 3])).toEqual(["7z"]);
    expect(normalizeSites(["http://[::1]/", "xn--pple-43d.com", "https://аpple.com"])).toEqual(["[::1]", "xn--pple-43d.com"]);
  });
});
