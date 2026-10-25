// The tray icon's tooltip (owner's request after #126): while downloads
// run, how many, how far they are together, and their speed, as
// «يعمل الآن: 3 تحميلات · 45% · 2 ميجابايت/ث». Built from the footer's
// own texts, in the user's language and digits.
import { invoke } from "@tauri-apps/api/core";
import type { Digits, Lang } from "./i18n.svelte";
import { percent, size, speed } from "./format";

type Download = { status: string; size: number | null; done: number; speed: number };
type T = (key: string, args?: Record<string, string | number>) => string;

/** The tooltip for `downloads`: the app's name, and a line about the
 * running ones if any. The percentage only when every running one's size
 * is known. */
export function trayTip(downloads: Download[], lang: Lang, digits: Digits, t: T): string {
  const active = downloads.filter((d) => d.status === "active");
  if (active.length === 0) return t("app-name");
  const parts = [t("footer-active", { count: active.length })];
  const total = active.reduce((sum, d) => sum + (d.size ?? 0), 0);
  if (total > 0 && active.every((d) => d.size !== null)) {
    parts.push(percent(lang, digits, active.reduce((sum, d) => sum + d.done, 0) / total));
  }
  const rate = active.reduce((sum, d) => sum + d.speed, 0);
  parts.push(speed(lang, digits, rate) || size(lang, digits, 0));
  return `${t("app-name")}\n${parts.join(" · ")}`;
}

let shown = "";

/** Sets the tooltip when it changed. */
export function showTrayTip(text: string): void {
  if (text === shown) return;
  shown = text;
  void invoke("set_tray_tooltip", { text }).catch(() => (shown = ""));
}
