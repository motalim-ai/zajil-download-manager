import { describe, expect, it, vi } from "vitest";
import { COOKIE_PERMISSIONS, askForCookies, cookieHeader, cookiesAllowed, dropCookies } from "../src/cookies.js";
import { downloadMessage, menuMessage, needsSignIn, takeOver } from "../src/capture.js";

/** A fake browser with the cookie permission `granted` or not, and these
 * cookies for every URL. */
function browserWith({ granted, cookies = [], getAllThrows = false }) {
  const asked = [];
  return {
    asked,
    permissions: {
      contains: vi.fn(async (p) => {
        expect(p).toEqual(COOKIE_PERMISSIONS);
        return granted;
      }),
      request: vi.fn(async (p) => {
        asked.push(p);
        granted = true;
        return true;
      }),
      remove: vi.fn(async () => {
        granted = false;
        return true;
      }),
    },
    cookies: {
      getAll: vi.fn(async ({ url }) => {
        if (getAllThrows) throw new Error("firstPartyDomain is required");
        expect(typeof url).toBe("string");
        return cookies;
      }),
    },
  };
}

describe("the cookie permission", () => {
  it("is the cookies API and every site's cookies, nothing else", () => {
    expect(COOKIE_PERMISSIONS).toEqual({ permissions: ["cookies"], origins: ["<all_urls>"] });
  });

  it("can be asked for, then taken back", async () => {
    const api = browserWith({ granted: false });
    expect(await cookiesAllowed(api)).toBe(false);
    expect(await askForCookies(api)).toBe(true);
    expect(api.asked).toEqual([COOKIE_PERMISSIONS]);
    expect(await cookiesAllowed(api)).toBe(true);
    await dropCookies(api);
    expect(await cookiesAllowed(api)).toBe(false);
  });
});

describe("with the permission", () => {
  const cookies = [
    { name: "sid", value: "abc123" },
    { name: "lang", value: "ar" },
  ];

  it("the link's cookies go in the message as one Cookie header", async () => {
    const api = browserWith({ granted: true, cookies });
    const header = await cookieHeader(api, "https://example.com/a.zip");
    expect(header).toBe("sid=abc123; lang=ar");
    expect(api.cookies.getAll).toHaveBeenCalledWith({ url: "https://example.com/a.zip" });
    expect(downloadMessage("https://example.com/a.zip", { cookies: header }).cookies).toBe("sid=abc123; lang=ar");
    expect(menuMessage({ linkUrl: "https://example.com/a.zip" }, header).cookies).toBe("sid=abc123; lang=ar");
  });

  it("a hand-over sends them", async () => {
    const sent = [];
    const api = {
      downloads: { pause: vi.fn(async () => {}), cancel: vi.fn(async () => {}), erase: vi.fn(async () => {}), resume: vi.fn(async () => {}) },
      runtime: { sendNativeMessage: vi.fn(async (_host, m) => (sent.push(m), { ok: true })) },
    };
    await takeOver(api, { id: 1, url: "https://example.com/a.zip", finalUrl: "https://example.com/a.zip" }, { pause: true, cookies: "sid=abc123" });
    expect(sent[0].cookies).toBe("sid=abc123");
  });

  it("a header the host would refuse is left out, so the download still goes", async () => {
    for (const bad of [
      [{ name: "a", value: "x\ny" }],
      [{ name: "a", value: "ز" }],
      [{ name: "a", value: "x".repeat(16 * 1024) }],
      [],
    ]) {
      expect(await cookieHeader(browserWith({ granted: true, cookies: bad }), "https://example.com/a.zip")).toBeNull();
    }
    expect(await cookieHeader(browserWith({ granted: true, getAllThrows: true }), "https://example.com/a.zip")).toBeNull();
  });
});

describe("without the permission", () => {
  it("no cookie is read, and the download goes without any", async () => {
    const api = browserWith({ granted: false, cookies: [{ name: "sid", value: "abc" }] });
    expect(await cookieHeader(api, "https://example.com/a.zip")).toBeNull();
    expect(api.cookies.getAll).not.toHaveBeenCalled();
    expect("cookies" in downloadMessage("https://example.com/a.zip", { cookies: null })).toBe(false);
    expect("cookies" in menuMessage({ linkUrl: "https://example.com/a.zip" }, null)).toBe(false);
  });

  it("a permission that cannot be checked counts as not given", async () => {
    const api = { permissions: { contains: async () => { throw new Error("no permissions API"); } } };
    expect(await cookiesAllowed(api)).toBe(false);
  });
});

describe("a site that wants a sign-in", () => {
  it("is told by the server's 401 or 403 to Zajil", () => {
    for (const status of ["401 Unauthorized", "403 Forbidden"]) {
      expect(needsSignIn({ outcome: "refused", error: `Zajil cannot download it: the server answered ${status}` })).toBe(true);
    }
    for (const result of [
      { outcome: "refused", error: "Zajil cannot download it: the server answered 404 Not Found" },
      { outcome: "refused", error: "Zajil cannot download it: the server sent a web page (a sign-in page?), not the file" },
      { outcome: "unreachable", error: "the server answered 403" },
      { outcome: "taken" },
      null,
    ]) {
      expect(needsSignIn(result), JSON.stringify(result)).toBe(false);
    }
  });
});
