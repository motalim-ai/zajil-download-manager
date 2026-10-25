import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
const { loadShow, saveShow } = await import("../src/lib/completeApi");
const { invoke } = await import("@tauri-apps/api/core");

function memory() {
  const kept = new Map<string, string>();
  return { getItem: (k: string) => kept.get(k) ?? null, setItem: (k: string, v: string) => void kept.set(k, v) };
}

describe("the «Download complete» window's choice", () => {
  it("shows unless the user turned it off, and comes back on", async () => {
    const storage = memory();
    expect(loadShow(storage)).toBe(true);
    await saveShow(false, storage);
    expect(loadShow(storage)).toBe(false);
    expect(invoke).toHaveBeenLastCalledWith("set_show_complete", { on: false });
    await saveShow(true, storage);
    expect(loadShow(storage)).toBe(true);
    expect(invoke).toHaveBeenLastCalledWith("set_show_complete", { on: true });
  });

  it("shows when nothing can be read", () => {
    expect(loadShow(undefined)).toBe(true);
    expect(loadShow({ getItem: () => { throw new Error("blocked"); } })).toBe(true);
  });
});
