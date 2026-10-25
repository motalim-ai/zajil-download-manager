// Numbers, sizes, speeds and times for display, in the user's digits:
// Western (123) by default, or Eastern Arabic (١٢٣), a setting.
import type { Digits, Lang } from "./i18n.svelte";
import { format } from "./i18n.svelte";

const KIB = 1024;
const MIB = 1024 * KIB;
const GIB = 1024 * MIB;

export function num(lang: Lang, digits: Digits, n: number, maxFraction = 1): string {
  return new Intl.NumberFormat(`${lang}-u-nu-${digits}`, { maximumFractionDigits: maxFraction }).format(n);
}

/// A size: `6.1 جيجابايت`, `512 ميجابايت`, `850 كيلوبايت`. The unit is
/// chosen after rounding, so 1,048,575 bytes is «1 MiB», never «1,024 KiB».
export function size(lang: Lang, digits: Digits, bytes: number): string {
  const rounded = (unit: number, fraction: number) => Math.round((bytes / unit) * 10 ** fraction) / 10 ** fraction;
  const mibFraction = bytes >= 100 * MIB ? 0 : 1;
  if (bytes >= GIB || rounded(MIB, mibFraction) >= 1024)
    return format(lang, "size-gib", { value: num(lang, digits, bytes / GIB) }, digits);
  if (bytes >= MIB || rounded(KIB, 0) >= 1024)
    return format(lang, "size-mib", { value: num(lang, digits, bytes / MIB, mibFraction) }, digits);
  if (bytes >= KIB) return format(lang, "size-kib", { value: num(lang, digits, bytes / KIB, 0) }, digits);
  return format(lang, "size-bytes", { value: num(lang, digits, bytes, 0) }, digits);
}

export function speed(lang: Lang, digits: Digits, bytesPerSecond: number): string {
  return bytesPerSecond > 0
    ? format(lang, "speed-per-second", { size: size(lang, digits, bytesPerSecond) }, digits)
    : "";
}

/// Whole percent, never 100 before the end.
export function percent(lang: Lang, digits: Digits, fraction: number): string {
  const whole = Math.floor(Math.min(Math.max(fraction, 0), 1) * 100);
  return new Intl.NumberFormat(`${lang}-u-nu-${digits}`, { style: "percent", maximumFractionDigits: 0 }).format(whole / 100);
}

/// What is left, in the largest unit: «9 دقائق», «2 ساعتان».
export function left(lang: Lang, digits: Digits, remainingBytes: number, bytesPerSecond: number): string {
  if (bytesPerSecond <= 0 || remainingBytes <= 0) return "";
  const seconds = remainingBytes / bytesPerSecond;
  if (seconds < 60) return counted(lang, digits, "seconds-left", Math.ceil(seconds));
  if (seconds < 3600) return counted(lang, digits, "minutes-left", Math.ceil(seconds / 60));
  return counted(lang, digits, "hours-left", Math.floor(seconds / 3600));
}

/// A count in its plural form, with the user's digits.
export function counted(lang: Lang, digits: Digits, key: string, n: number): string {
  return format(lang, key, { count: n }, digits);
}

/** «Last try» (#95): the time today, «yesterday» and the time, else the
 * date; in the user's local time. `when`: RFC 3339. */
export function lastTry(
  lang: Lang,
  digits: Digits,
  when: string,
  t: (key: string, args?: Record<string, string | number>) => string,
  now: Date = new Date(),
): string {
  const at = new Date(when);
  if (Number.isNaN(at.getTime())) return "";
  const locale = `${lang}-u-nu-${digits}`;
  const time = new Intl.DateTimeFormat(locale, { hour: "2-digit", minute: "2-digit", hourCycle: "h23" }).format(at);
  const day = (d: Date) => new Date(d.getFullYear(), d.getMonth(), d.getDate()).getTime();
  const days = Math.round((day(now) - day(at)) / 86_400_000);
  if (days === 0) return time;
  if (days === 1) return t("last-try-yesterday", { time });
  return new Intl.DateTimeFormat(locale, { day: "numeric", month: "short" }).format(at);
}
