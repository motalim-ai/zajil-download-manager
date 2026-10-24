// The interface's translations (ADR-0008): the same Fluent files as the
// Rust side (`locales/`, crate `zajil-i18n`), through @fluent/bundle.
// Changing the language changes every text at once, without a restart.
import { FluentBundle, FluentResource, type FluentVariable } from "@fluent/bundle";
import ar from "../../../../locales/ar/zajil.ftl?raw";
import en from "../../../../locales/en/zajil.ftl?raw";

export type Lang = "ar" | "en";
export type Digits = "latn" | "arab";
export const LANGS: Lang[] = ["ar", "en"];
const SOURCES: Record<Lang, string> = { ar, en };

/// The CLDR plural category of `n` (zero, one, two, few, many, other), for
/// `PLURAL($count)` in the messages; Rust uses ICU4X for the same function.
export function pluralCategory(lang: Lang, n: number): string {
  return new Intl.PluralRules(lang).select(n);
}

/// `text` without the characters Rust's `str::trim` removes (Unicode
/// White_Space): JavaScript's `trim` also removes U+FEFF, which Rust keeps,
/// so «\uFEFF3» would be 3 here and refused there (review of #45).
const WHITE_SPACE = "[\\t-\\r \\u0085\\u00A0\\u1680\\u2000-\\u200A\\u2028\\u2029\\u202F\\u205F\\u3000]";
const TRIM = new RegExp(`^${WHITE_SPACE}+|${WHITE_SPACE}+$`, "g");
function trimWhiteSpace(text: string): string {
  return text.replace(TRIM, "");
}

/// A count given as text, read as Rust's `zajil-i18n` reads it: digits
/// (Western, Eastern Arabic or Persian), optionally in groups of three after
/// a thousands separator, and a decimal point (`.` or `٫`). Anything else
/// (`1,5`, `1e3`, `inf`, `-3`) is not a count (review of #43).
export function parseCount(text: string): number | null {
  const plain = [...trimWhiteSpace(text)]
    .map((c) => {
      const code = c.codePointAt(0) ?? 0;
      if (code >= 0x0660 && code <= 0x0669) return String(code - 0x0660);
      if (code >= 0x06f0 && code <= 0x06f9) return String(code - 0x06f0);
      return c === "\u066B" ? "." : c;
    })
    .join("");
  const [whole = "", fraction, ...rest] = plain.split(".");
  if (rest.length > 0) return null;
  const groups = whole.split(/[,\u066C\u00A0\u202F\u2009]/);
  const digits = (g: string) => /^[0-9]+$/.test(g);
  const grouped = groups.length === 1 || ((groups[0] ?? "").length <= 3 && groups.slice(1).every((g) => g.length === 3));
  if (!grouped || !groups.every(digits) || (fraction !== undefined && !digits(fraction))) return null;
  return Number(groups.join("") + (fraction === undefined ? "" : `.${fraction}`));
}

/// A value without direction marks, and with its controls and line breaks
/// as spaces, as in Rust: its own U+2069 would end the isolation Fluent
/// puts around it (review of #43).
export function cleanValue(text: string): string {
  return text
    .replace(/[\u202A-\u202E\u2066-\u2069\u061C\u200E\u200F]/g, "")
    .replace(/[\u0000-\u001F\u007F-\u009F\u2028\u2029]/g, " ");
}

/// A bundle for `lang` whose numbers use `digits` (Western 123 or Eastern
/// Arabic ١٢٣, a setting).
export function makeBundle(lang: Lang, digits: Digits = "latn"): FluentBundle {
  const bundle = new FluentBundle(`${lang}-u-nu-${digits}`, {
    functions: {
      // A count that is not a plain, finite, non-negative number is an
      // error (thrown, so Fluent records it), never the default form.
      PLURAL: ([value]) => {
        const raw = (value as { valueOf(): unknown } | undefined)?.valueOf();
        const n = typeof raw === "number" ? raw : typeof raw === "string" ? parseCount(raw) : null;
        if (n === null || !Number.isFinite(n) || n < 0) throw new RangeError(`not a count: ${String(raw)}`);
        return pluralCategory(lang, n);
      },
    },
  });
  const errors = bundle.addResource(new FluentResource(SOURCES[lang]));
  if (errors.length > 0) throw new Error(`${lang}: ${errors.join("; ")}`);
  return bundle;
}

const bundles = new Map<string, FluentBundle>();

function bundleFor(lang: Lang, digits: Digits): FluentBundle {
  const id = `${lang}-${digits}`;
  let bundle = bundles.get(id);
  if (!bundle) {
    bundle = makeBundle(lang, digits);
    bundles.set(id, bundle);
  }
  return bundle;
}

/// Formats `key` in `lang`. A missing key, or a value that is not a finite
/// number or not a count where one is needed, shows the key itself, so a
/// mistake is visible rather than wrong (the translation tests catch it).
export function format(
  lang: Lang,
  key: string,
  args?: Record<string, FluentVariable>,
  digits: Digits = "latn",
): string {
  const bundle = bundleFor(lang, digits);
  const message = bundle.getMessage(key);
  if (!message?.value) return key;
  const clean: Record<string, FluentVariable> = {};
  for (const [name, value] of Object.entries(args ?? {})) {
    if (typeof value === "number") {
      if (!Number.isFinite(value)) return key;
      clean[name] = value + 0; // -0 as 0
    } else {
      clean[name] = typeof value === "string" ? cleanValue(value) : value;
    }
  }
  const errors: Error[] = [];
  const text = bundle.formatPattern(message.value, clean, errors);
  return errors.length > 0 ? key : text;
}

class Locale {
  lang = $state<Lang>("ar");
  digits = $state<Digits>("latn");

  get dir(): "rtl" | "ltr" {
    return this.lang === "ar" ? "rtl" : "ltr";
  }

  t = (key: string, args?: Record<string, FluentVariable>): string =>
    format(this.lang, key, args, this.digits);

  set(lang: Lang): void {
    this.lang = lang;
    document.documentElement.lang = lang;
    document.documentElement.dir = this.dir;
  }
}

export const locale = new Locale();
