//! Zajil's translations (M4, ADR-0003): Fluent files in `locales/`, one
//! per language, shared by the GUI (which loads the same files with
//! `@fluent/bundle`, ADR-0008) and the Rust side (the tray menu and the
//! notifications). The command line stays in English (owner's decision on
//! the M4 plan).
//!
//! Plural forms follow the CLDR rules of each language: Arabic has six
//! (zero, one, two, few, many, other). Messages select them with
//! `PLURAL($count)`, a function that returns the CLDR category's name: here
//! from ICU4X (`icu_plurals`), in the GUI from `Intl.PluralRules`. Not with
//! `$count` itself: in Rust that would use the rules `fluent-bundle` ships
//! (`intl_pluralrules` 7.0.2), which put Arabic 103 and 111 in "other"
//! (found by this crate's tests).

#![forbid(unsafe_code)]

use fluent_bundle::{FluentArgs, FluentBundle, FluentResource, FluentValue};
use unic_langid::LanguageIdentifier;

/// The languages Zajil ships, Arabic first.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Lang {
    Ar,
    En,
}

impl Lang {
    pub const ALL: [Lang; 2] = [Lang::Ar, Lang::En];

    /// The BCP 47 tag: `ar` or `en`.
    pub fn tag(self) -> &'static str {
        match self {
            Lang::Ar => "ar",
            Lang::En => "en",
        }
    }

    /// Whether text in this language runs from right to left.
    pub fn is_rtl(self) -> bool {
        matches!(self, Lang::Ar)
    }

    /// The Fluent source of this language (`locales/<tag>/zajil.ftl`).
    pub fn source(self) -> &'static str {
        match self {
            Lang::Ar => include_str!("../../../locales/ar/zajil.ftl"),
            Lang::En => include_str!("../../../locales/en/zajil.ftl"),
        }
    }
}

/// Formats the messages of one language.
pub struct Localizer {
    lang: Lang,
    bundle: FluentBundle<FluentResource>,
}

impl std::fmt::Debug for Localizer {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Localizer")
            .field("lang", &self.lang)
            .finish()
    }
}

/// A message is missing, or the file has an error (both are caught by the
/// tests before a release).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MissingMessage(pub String);

impl Localizer {
    /// The messages of `lang`. Values put into a message are wrapped in
    /// Unicode isolation marks (U+2068 … U+2069), so a Latin file name
    /// inside an Arabic sentence keeps its own direction.
    pub fn new(lang: Lang) -> Self {
        let id: LanguageIdentifier = lang.tag().parse().expect("a valid language tag");
        let mut bundle = FluentBundle::new(vec![id]);
        let resource = FluentResource::try_new(lang.source().to_owned())
            .unwrap_or_else(|(resource, _)| resource);
        // Errors (a repeated key) are caught by the tests; the first
        // definition wins here.
        let _ = bundle.add_resource(resource);
        let tag = lang.tag();
        bundle
            .add_function("PLURAL", move |positional, _named| {
                // A number, or text that is a number (as in the GUI, where
                // Intl.PluralRules reads "3" as 3: review of #43).
                let n = match positional.first() {
                    Some(FluentValue::Number(n)) => Some(n.value),
                    Some(FluentValue::String(s)) => parse_count(s),
                    _ => None,
                };
                match n {
                    // A count is never negative: «-1» would show as «تحميل
                    // واحد», since the one and two forms have no number
                    // (review of #43, round 5).
                    Some(n) if n.is_finite() && n >= 0.0 => {
                        FluentValue::from(plural_category(tag, n))
                    }
                    _ => {
                        // Fluent would quietly pick the default form:
                        // `text` fails instead (review of #43, round 4).
                        UNREADABLE_COUNT.with(|c| c.set(true));
                        FluentValue::Error
                    }
                }
            })
            .expect("PLURAL is defined once");
        Localizer { lang, bundle }
    }

    pub fn lang(&self) -> Lang {
        self.lang
    }

    /// The message `key`, with `args`.
    pub fn text(&self, key: &str, args: &[(&str, Arg<'_>)]) -> Result<String, MissingMessage> {
        let message = self
            .bundle
            .get_message(key)
            .ok_or_else(|| MissingMessage(key.to_owned()))?;
        let pattern = message
            .value()
            .ok_or_else(|| MissingMessage(key.to_owned()))?;
        let mut fluent_args = FluentArgs::new();
        for (name, value) in args {
            // `inf` or `NaN` in a sentence is a mistake (round 7).
            if let Arg::Number(n) = value
                && !n.is_finite()
            {
                return Err(MissingMessage(format!(
                    "{key}: ${name} is not a finite number"
                )));
            }
            match value {
                // `+ 0.0` turns -0 into 0, which would show as «-0».
                Arg::Number(n) => fluent_args.set(*name, FluentValue::from(*n + 0.0)),
                Arg::Text(t) => fluent_args.set(*name, FluentValue::from(without_bidi_controls(t))),
            }
        }
        let mut errors = Vec::new();
        UNREADABLE_COUNT.with(|c| c.set(false));
        let text = self
            .bundle
            .format_pattern(pattern, Some(&fluent_args), &mut errors);
        if UNREADABLE_COUNT.with(|c| c.replace(false)) {
            return Err(MissingMessage(format!(
                "{key}: a count that is not a number"
            )));
        }
        if errors.is_empty() {
            Ok(text.into_owned())
        } else {
            Err(MissingMessage(format!("{key}: {errors:?}")))
        }
    }
}

/// A value for a message.
#[derive(Debug, Clone, Copy)]
pub enum Arg<'a> {
    Number(f64),
    Text(&'a str),
}

/// The CLDR cardinal plural category of `n` in language `tag`: `zero`,
/// `one`, `two`, `few`, `many` or `other`. Fractions are `other` in Arabic
/// and English (CLDR: a visible fraction is never one or few).
pub fn plural_category(tag: &str, n: f64) -> &'static str {
    use icu_plurals::{PluralCategory, PluralRules};
    let Ok(locale) = tag.parse::<icu_locale_core::Locale>() else {
        return "other";
    };
    let Ok(rules) = PluralRules::try_new_cardinal(locale.into()) else {
        return "other";
    };
    if !n.is_finite() || n.fract() != 0.0 {
        return "other";
    }
    // Every f64 beyond 2^53 is a whole number. The Arabic and English rules
    // look at the last two digits only, so beyond u64 the number is cut to
    // its last six (exact in f64) instead of saturating (review of #43).
    let n = n.abs();
    let n = if n >= 1e19 { n % 1e6 + 1e6 } else { n };
    match rules.category_for(n as u64) {
        PluralCategory::Zero => "zero",
        PluralCategory::One => "one",
        PluralCategory::Two => "two",
        PluralCategory::Few => "few",
        PluralCategory::Many => "many",
        PluralCategory::Other => "other",
    }
}

thread_local! {
    /// Set by `PLURAL` when its count is not a number, during one
    /// `format_pattern` on this thread.
    static UNREADABLE_COUNT: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// A count given as text: digits (Western, Eastern Arabic `٣` or Persian
/// `۳`: review of #43, round 3), optionally in groups of three after
/// thousands separators (`1,003`, `١٬٠٠٣`) and with a decimal point (`.` or
/// `٫`). Anything else, such as `1,5`, `1e3`, `inf` or `-3`, is not a
/// count (round 5).
fn parse_count(text: &str) -> Option<f64> {
    let plain: String = text
        .trim()
        .chars()
        .map(|c| match c {
            '\u{0660}'..='\u{0669}' => char::from(b'0' + (c as u32 - 0x0660) as u8),
            '\u{06F0}'..='\u{06F9}' => char::from(b'0' + (c as u32 - 0x06F0) as u8),
            '\u{066B}' => '.',
            _ => c,
        })
        .collect();
    let (whole, fraction) = match plain.split_once('.') {
        Some((w, f)) => (w, Some(f)),
        None => (plain.as_str(), None),
    };
    let digits = |g: &str| !g.is_empty() && g.bytes().all(|b| b.is_ascii_digit());
    let groups: Vec<&str> = whole
        .split([',', '\u{066C}', '\u{00A0}', '\u{202F}', '\u{2009}'])
        .collect();
    let grouped =
        groups.len() == 1 || (groups[0].len() <= 3 && groups[1..].iter().all(|g| g.len() == 3));
    if !(grouped && groups.iter().all(|g| digits(g)) && fraction.is_none_or(digits)) {
        return None;
    }
    let mut number = groups.concat();
    if let Some(f) = fraction {
        number.push('.');
        number.push_str(f);
    }
    number.parse().ok()
}

/// A value without the marks that open or close a direction (U+202A to
/// U+202E, U+2066 to U+2069, and the plain marks U+061C, U+200E, U+200F),
/// and with its controls and line breaks as spaces: its own U+2069 would end the isolation the
/// message puts around it, and a U+202E after that would reverse what
/// follows (review of #43, round 4). File names are already cleaned by
/// `zajil_core::sanitize`; a reason, a site or a folder is not.
fn without_bidi_controls(text: &str) -> String {
    text.chars()
        .filter_map(|c| match c {
            // Direction marks go.
            '\u{202A}'..='\u{202E}'
            | '\u{2066}'..='\u{2069}'
            | '\u{061C}'
            | '\u{200E}'
            | '\u{200F}' => None,
            // Controls, tab included (a segment separator that reorders an
            // Arabic sentence), and line and paragraph breaks become a
            // space, so words stay apart (round 7); U+0085 and U+001C to
            // U+001E end a paragraph too (round 6).
            c if c.is_control() || matches!(c, '\u{2028}' | '\u{2029}') => Some(' '),
            c => Some(c),
        })
        .collect()
}

/// Removes the isolation marks Fluent puts around values, for comparing
/// text in tests.
pub fn without_isolation(text: &str) -> String {
    text.chars()
        .filter(|c| *c != '\u{2068}' && *c != '\u{2069}')
        .collect()
}
