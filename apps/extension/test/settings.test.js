import { describe, expect, it } from "vitest";
import { DEFAULT_BIG_FILE_MB, DEFAULT_EXTENSIONS, KEY_CHOICES, MAX_BIG_FILE_MB, clickKeys, defaultKeys, defaults, isMac, load, normalizeExtensions, normalizeSites, save } from "../src/settings.js";
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
    expect(await load(api)).toEqual({ capture: false, extensions: ["zip", "mp4"], bigFiles: false, bigFileMb: 5, excludedSites: ["example.com", "sub.test"], useKeys: false, sendKeys: null, browserKeys: null });
    expect(await load(fakeStorage({ capture: "yes", extensions: "zip", excludedSites: [42] }))).toEqual({ ...defaults(), excludedSites: [] });
  });

  it("take any file larger than 5 MB from the start, and keep a size from 1 to 100,000 MB", async () => {
    expect(defaults()).toMatchObject({ bigFiles: true, bigFileMb: DEFAULT_BIG_FILE_MB });
    expect(DEFAULT_BIG_FILE_MB).toBe(5);
    const api = fakeStorage();
    await save(api, { ...defaults(), bigFiles: false, bigFileMb: 250 });
    expect(await load(api)).toMatchObject({ bigFiles: false, bigFileMb: 250 });
    for (const odd of [0, -3, 2.5, MAX_BIG_FILE_MB + 1, NaN, "40", null]) {
      await save(api, { ...defaults(), bigFileMb: odd });
      expect((await load(api)).bigFileMb).toBe(DEFAULT_BIG_FILE_MB);
      expect(await load(fakeStorage({ bigFileMb: odd, bigFiles: "no" }))).toMatchObject({ bigFiles: true, bigFileMb: DEFAULT_BIG_FILE_MB });
    }
    await save(api, { ...defaults(), bigFileMb: MAX_BIG_FILE_MB });
    expect((await load(api)).bigFileMb).toBe(MAX_BIG_FILE_MB);
  });

  it("normalize odd entries away", () => {
    expect(normalizeExtensions(["7Z", "tar.gz", null, 3])).toEqual(["7z"]);
    expect(normalizeSites(["http://[::1]/", "xn--pple-43d.com", "https://аpple.com"])).toEqual(["[::1]", "xn--pple-43d.com"]);
  });
  it("the keys of a click: the owner's defaults for each system, any choice kept, odd ones dropped", async () => {
    expect(defaultKeys(false)).toEqual({ send: "ctrl", browser: "alt" });
    expect(defaultKeys(true)).toEqual({ send: "alt", browser: "alt+shift" });
    expect(isMac({ platform: "MacIntel" })).toBe(true);
    expect(isMac({ userAgentData: { platform: "macOS" } })).toBe(true);
    expect(isMac({ platform: "Win32" })).toBe(false);
    expect(isMac({ platform: "Linux x86_64" })).toBe(false);
    expect(clickKeys(defaults(), false)).toEqual({ send: "ctrl", browser: "alt" });
    expect(clickKeys(defaults(), true)).toEqual({ send: "alt", browser: "alt+shift" });
    // The same keys for both: Zajil's win.
    expect(clickKeys({ sendKeys: "alt", browserKeys: null }, false)).toEqual({ send: "alt", browser: "off" });
    const api = fakeStorage();
    await save(api, { ...defaults(), sendKeys: "ctrl+shift", browserKeys: "meta" });
    expect(await load(api)).toMatchObject({ sendKeys: "ctrl+shift", browserKeys: null });
    for (const choice of KEY_CHOICES) {
      await save(api, { ...defaults(), sendKeys: choice });
      expect((await load(api)).sendKeys).toBe(choice);
    }
    // Never Cmd, the Windows key or Shift alone.
    expect(KEY_CHOICES.some((c) => /meta|cmd|win/.test(c) || c === "shift")).toBe(false);
  });

  it("click-keys.js, which cannot import this module, has the same choices and defaults", () => {
    const script = readFileSync(new URL("../src/click-keys.js", import.meta.url), "utf8");
    expect(script).toContain(`const CHOICES = ${JSON.stringify(KEY_CHOICES).replaceAll(",", ", ")};`);
    expect(script).toContain(`mac ? { send: "${defaultKeys(true).send}", browser: "${defaultKeys(true).browser}" } : { send: "${defaultKeys(false).send}", browser: "${defaultKeys(false).browser}" }`);
  });
  it("the keys of a click are off by default, as in IDM, until the user turns them on (owner's decision)", async () => {
    expect(defaults().useKeys).toBe(false);
    expect((await load(fakeStorage({ useKeys: "yes" }))).useKeys).toBe(false);
    const api = fakeStorage();
    await save(api, { ...defaults(), useKeys: true });
    expect((await load(api)).useKeys).toBe(true);
  });
});
