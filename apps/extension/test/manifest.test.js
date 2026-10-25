import { describe, expect, it } from "vitest";
import { createHash } from "node:crypto";
import { readFileSync } from "node:fs";
import { HOST, MAX_NAME, MAX_URL, NO_ANSWER, PROTOCOL_VERSION } from "../src/capture.js";

const read = (path) => readFileSync(new URL(path, import.meta.url), "utf8");
const chrome = JSON.parse(read("../src/manifest.chrome.json"));
const firefox = JSON.parse(read("../src/manifest.firefox.json"));
const ids = read("../../../crates/zajil-platform/src/ids.rs");
const host = read("../../../crates/zajil-host/src/lib.rs");
const hostMain = read("../../../crates/zajil-host/src/main.rs");
const rustConst = (source, name) => source.match(new RegExp(`pub const ${name}: [^=]+= ([^;]+);`))?.[1];

/** Chrome's id: the first 32 hex digits of SHA-256 of the public key (DER),
 * each written a–p. */
function chromeId(key) {
  const hex = createHash("sha256").update(Buffer.from(key, "base64")).digest("hex").slice(0, 32);
  return [...hex].map((c) => String.fromCharCode(97 + parseInt(c, 16))).join("");
}

describe("the manifests", () => {
  it("give the fixed ids Zajil registers (owner's decision on #7)", () => {
    expect(chromeId(chrome.key)).toBe(rustConst(ids, "EXTENSION_ID_CHROMIUM").replaceAll('"', ""));
    expect(firefox.browser_specific_settings.gecko.id).toBe(rustConst(ids, "EXTENSION_ID_FIREFOX").replaceAll('"', ""));
    expect(firefox.key).toBeUndefined();
  });

  it("hold a public key only", () => {
    const der = Buffer.from(chrome.key, "base64");
    // SubjectPublicKeyInfo for RSA: SEQUENCE, then the rsaEncryption OID.
    expect(der[0]).toBe(0x30);
    expect(der.includes(Buffer.from("2a864886f70d010101", "hex"))).toBe(true);
    expect(der.length).toBeLessThan(400);
  });

  it("ask for the same few permissions, and no site", () => {
    const wanted = ["downloads", "contextMenus", "nativeMessaging", "storage", "notifications"];
    for (const m of [chrome, firefox]) {
      expect(m.manifest_version).toBe(3);
      expect([...m.permissions].sort()).toEqual([...wanted].sort());
      expect(m.host_permissions).toBeUndefined();
      expect(m.optional_permissions).toBeUndefined();
      expect(m.content_scripts).toBeUndefined();
    }
    const { key: _key, background: cb, minimum_chrome_version: _min, ...chromeRest } = chrome;
    const { browser_specific_settings: _bss, background: fb, ...firefoxRest } = firefox;
    expect(chromeRest).toEqual(firefoxRest);
    expect(cb).toEqual({ service_worker: "background.js", type: "module" });
    expect(fb).toEqual({ scripts: ["background.js"], type: "module" });
  });

  it("have every text in Arabic and English", () => {
    const ar = JSON.parse(read("../src/_locales/ar/messages.json"));
    const en = JSON.parse(read("../src/_locales/en/messages.json"));
    expect(Object.keys(ar).sort()).toEqual(Object.keys(en).sort());
    const used = new Set();
    for (const file of ["background.js", "options.html", "manifest.chrome.json"]) {
      for (const m of read(`../src/${file}`).matchAll(/(?:__MSG_(\w+)__|data-i18n="(\w+)"|t\("(\w+)"\)|: "(not[A-Z]\w+)")/g)) {
        used.add(m[1] || m[2] || m[3] || m[4]);
      }
    }
    for (const key of ["notRefused", "notAnswered", "notReachable", "menuDownloadLink"]) expect(used.has(key), key).toBe(true);
    for (const key of used) expect(ar[key], key).toBeDefined();
  });
});

describe("the protocol, as zajil-host has it", () => {
  it("has the same host name, version and limits", () => {
    expect(rustConst(ids, "NATIVE_HOST_NAME")).toBe(`"${HOST}"`);
    expect(Number(rustConst(host, "PROTOCOL_VERSION"))).toBe(PROTOCOL_VERSION);
    expect(rustConst(host, "MAX_URL")).toBe("8 * 1024");
    expect(MAX_URL).toBe(8 * 1024);
    expect(Number(rustConst(host, "MAX_NAME"))).toBe(MAX_NAME);
    expect(hostMain).toContain(`"${NO_ANSWER} {} s"`);
  });
});
