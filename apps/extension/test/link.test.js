import { describe, expect, it, vi } from "vitest";
import { answer, instanceId, knownSite, MAX_RETRY_MS, plainStore, rememberSite, SITE_DAYS, startLink } from "../src/link.js";

/** A fake browser: `storage.local`, the cookie permission, the cookie
 * jar (by store), and native ports. */
function fake({ granted = true } = {}) {
  const local = {};
  const reads = [];
  const ports = [];
  const on = () => {
    const listeners = [];
    return { addListener: (f) => listeners.push(f), fire: (...a) => listeners.forEach((f) => f(...a)) };
  };
  const api = {
    storage: {
      local: {
        get: async (key) => ({ [key]: local[key] }),
        set: async (value) => Object.assign(local, value),
      },
    },
    permissions: { contains: async () => granted },
    cookies: {
      getAll: async (query) => {
        reads.push(query);
        return [{ name: "sid", value: query.storeId ?? "default" }];
      },
    },
    runtime: {
      connectNative: vi.fn(() => {
        const port = { sent: [], onMessage: on(), onDisconnect: on(), postMessage: (m) => port.sent.push(m) };
        ports.push(port);
        return port;
      }),
    },
  };
  return { api, local, reads, ports };
}

const flush = () => new Promise((r) => setTimeout(r, 10));
const wanted = (extra = {}) => ({ type: "cookies_wanted", protocol_version: 1, ask: 7, url: "https://files.example.com/a.zip", ...extra });

describe("the link to Zajil (M5)", () => {
  it("has one instance id per profile, made once", async () => {
    const { api } = fake();
    const first = await instanceId(api);
    expect(first).toMatch(/^[0-9a-f-]{36}$/);
    expect(await instanceId(api)).toBe(first);
  });

  it("answers only for a site it sent to Zajil with cookies, in that store", async () => {
    const { api, reads } = fake();
    // Never sent: nothing read, no cookies.
    expect(await answer(api, wanted())).toEqual({ type: "cookies", protocol_version: 1, ask: 7, cookies: null });
    expect(reads).toEqual([]);
    await rememberSite(api, "https://files.example.com/other/b.zip", "firefox-container-1");
    // The same site and store: its cookies, of the link asked about.
    expect((await answer(api, wanted({ store: "firefox-container-1" }))).cookies).toBe("sid=firefox-container-1");
    expect(reads).toEqual([{ url: "https://files.example.com/a.zip", storeId: "firefox-container-1" }]);
    // Another store, another site, a private window's store: none.
    expect((await answer(api, wanted())).cookies).toBeNull();
    expect((await answer(api, wanted({ url: "https://bank.example.com/", store: "firefox-container-1" }))).cookies).toBeNull();
    expect((await answer(api, wanted({ store: "firefox-private" }))).cookies).toBeNull();
    expect(reads).toHaveLength(1);
  });

  it("gives no cookies without the permission, and forgets sites after 30 days", async () => {
    const { api } = fake({ granted: false });
    await rememberSite(api, "https://files.example.com/a.zip", undefined);
    expect((await answer(api, wanted())).cookies).toBeNull();
    const old = Date.now() - (SITE_DAYS + 1) * 24 * 3600 * 1000;
    const { api: api2 } = fake();
    await rememberSite(api2, "https://files.example.com/a.zip", undefined, old);
    expect(await knownSite(api2, "https://files.example.com/a.zip", undefined)).toBe(false);
  });

  it("keeps no cookie and no site in clear", async () => {
    const { api, local } = fake();
    await rememberSite(api, "https://files.example.com/a.zip", undefined);
    expect(JSON.stringify(local)).not.toContain("example");
  });

  it("is not a question: no answer; a bad link or store: no cookies", async () => {
    const { api } = fake();
    expect(await answer(api, { type: "other", ask: 1 })).toBeNull();
    expect(await answer(api, wanted({ ask: -1 }))).toBeNull();
    await rememberSite(api, "https://files.example.com/a.zip", undefined);
    expect((await answer(api, wanted({ url: "javascript:alert(1)" }))).cookies).toBeNull();
    expect((await answer(api, wanted({ store: "bad store!" }))).cookies).toBeNull();
    expect(plainStore("firefox-container-3")).toBe("firefox-container-3");
    expect(plainStore("firefox-private")).toBeUndefined();
  });

  it("opens the link, answers on it, and opens it again when it closes", async () => {
    const { api, ports } = fake();
    await rememberSite(api, "https://files.example.com/a.zip", undefined);
    const waits = [];
    startLink(api, { wait: (f, ms) => waits.push({ f, ms }) });
    await flush();
    expect(ports).toHaveLength(1);
    expect(ports[0].sent[0]).toMatchObject({ type: "link", protocol_version: 1, instance: await instanceId(api) });
    ports[0].onMessage.fire(wanted());
    await flush();
    expect(ports[0].sent[1]).toEqual({ type: "cookies", protocol_version: 1, ask: 7, cookies: "sid=default" });
    // Closed: opened again after a wait that grows, up to a minute.
    for (let i = 0; i < 8; i++) {
      ports.at(-1).onDisconnect.fire();
      waits.at(-1).f();
      await flush();
    }
    expect(ports).toHaveLength(9);
    expect(waits.map((w) => w.ms)).toEqual([2000, 4000, 8000, 16000, 32000, MAX_RETRY_MS, MAX_RETRY_MS, MAX_RETRY_MS]);
  });
});
