import { describe, expect, it, vi } from "vitest";

vi.mock("@tauri-apps/api/core", () => ({ invoke: vi.fn(async () => undefined) }));
const { trayTip } = await import("../src/lib/trayTip");
const { format } = await import("../src/lib/i18n.svelte");

const t = (lang: "ar" | "en") => (key: string, args?: Record<string, string | number>) => format(lang, key, args ?? {}, "latn");
const d = (status: string, size: number | null, done: number, speed: number) => ({ status, size, done, speed });
const strip = (s: string) => s.replace(/[⁦-⁩]/g, "");

describe("the tray icon's tooltip", () => {
  it("says how many run, how far they are together and their speed", () => {
    const downloads = [d("active", 1000, 500, 1048576), d("active", 1000, 400, 1048576), d("active", 2000, 900, 0), d("waiting", 5000, 0, 0), d("done", 10, 10, 0)];
    expect(strip(trayTip(downloads, "en", "latn", t("en")))).toBe("Zajil\nDownloading: 3 downloads · 45% · 2 MiB/s");
    expect(strip(trayTip(downloads, "ar", "latn", t("ar")))).toContain("يعمل الآن: 3 تحميلات · 45");
  });

  it("leaves out the percentage when a size is unknown", () => {
    const text = strip(trayTip([d("active", null, 500, 2048), d("active", 1000, 10, 0)], "en", "latn", t("en")));
    expect(text).not.toContain("%");
    expect(text).toContain("Downloading: 2 downloads");
  });

  it("is the app's name when nothing runs", () => {
    expect(trayTip([d("paused", 10, 1, 0)], "en", "latn", t("en"))).toBe(t("en")("app-name"));
  });
});
