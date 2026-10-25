//! M4 success criteria: every translation key exists in Arabic and in
//! English (with the same variables, and no syntax error), and the Arabic
//! plural forms are right for 0, 1, 2, 3, 11 and 100.

use std::collections::{BTreeMap, BTreeSet};

use fluent_syntax::ast::{
    Entry, Expression, InlineExpression, Pattern, PatternElement, VariantKey,
};
use fluent_syntax::parser;
use zajil_i18n::{Arg, Lang, Localizer, without_isolation};

/// Each message of a file, with the variables it uses (in its value and
/// its attributes, in every variant). Panics on any syntax error.
fn messages(lang: Lang) -> BTreeMap<String, BTreeSet<String>> {
    let resource = parser::parse(lang.source())
        .unwrap_or_else(|(_, errors)| panic!("{}: syntax errors: {errors:?}", lang.tag()));
    let mut out = BTreeMap::new();
    for entry in resource.body {
        match entry {
            Entry::Message(m) => {
                let mut vars = BTreeSet::new();
                if let Some(value) = &m.value {
                    variables(value, &mut vars);
                }
                for attribute in &m.attributes {
                    variables(&attribute.value, &mut vars);
                }
                let name = m.id.name.to_owned();
                assert!(
                    out.insert(name.clone(), vars).is_none(),
                    "{}: {name} is defined twice",
                    lang.tag()
                );
            }
            Entry::Term(t) => panic!("{}: terms are not used ({})", lang.tag(), t.id.name),
            Entry::Junk { content } => panic!("{}: junk: {content}", lang.tag()),
            _ => {}
        }
    }
    out
}

fn variables(pattern: &Pattern<&str>, out: &mut BTreeSet<String>) {
    for element in &pattern.elements {
        if let PatternElement::Placeable { expression } = element {
            expression_variables(expression, out);
        }
    }
}

fn expression_variables(expression: &Expression<&str>, out: &mut BTreeSet<String>) {
    match expression {
        Expression::Inline(inline) => inline_variables(inline, out),
        Expression::Select { selector, variants } => {
            inline_variables(selector, out);
            for variant in variants {
                variables(&variant.value, out);
            }
        }
    }
}

fn inline_variables(inline: &InlineExpression<&str>, out: &mut BTreeSet<String>) {
    match inline {
        InlineExpression::VariableReference { id } => {
            out.insert(id.name.to_owned());
        }
        InlineExpression::Placeable { expression } => expression_variables(expression, out),
        InlineExpression::FunctionReference { arguments, .. } => {
            for arg in &arguments.positional {
                inline_variables(arg, out);
            }
            for arg in &arguments.named {
                inline_variables(&arg.value, out);
            }
        }
        InlineExpression::MessageReference { .. }
        | InlineExpression::TermReference { .. }
        | InlineExpression::StringLiteral { .. }
        | InlineExpression::NumberLiteral { .. } => {}
    }
}

#[test]
fn every_key_exists_in_arabic_and_in_english_with_the_same_variables() {
    let ar = messages(Lang::Ar);
    let en = messages(Lang::En);
    let only_ar: Vec<_> = ar.keys().filter(|k| !en.contains_key(*k)).collect();
    let only_en: Vec<_> = en.keys().filter(|k| !ar.contains_key(*k)).collect();
    assert!(only_ar.is_empty(), "missing in English: {only_ar:?}");
    assert!(only_en.is_empty(), "missing in Arabic: {only_en:?}");
    for (key, vars) in &ar {
        assert_eq!(vars, &en[key], "{key}: the variables differ");
    }
    assert!(ar.len() > 100, "only {} keys", ar.len());
}

#[test]
fn every_message_formats_without_errors_in_both_languages() {
    for lang in Lang::ALL {
        let l10n = Localizer::new(lang);
        for (key, vars) in messages(lang) {
            let args: Vec<(&str, Arg<'_>)> = vars
                .iter()
                .map(|v| {
                    let value = if v == "count" {
                        Arg::Number(3.0)
                    } else {
                        Arg::Text("x")
                    };
                    (v.as_str(), value)
                })
                .collect();
            let text = l10n
                .text(&key, &args)
                .unwrap_or_else(|e| panic!("{}: {e:?}", lang.tag()));
            assert!(!text.trim().is_empty(), "{}: {key} is empty", lang.tag());
        }
    }
}

fn text(lang: Lang, key: &str, count: f64) -> String {
    without_isolation(
        &Localizer::new(lang)
            .text(
                key,
                &[("count", Arg::Number(count)), ("percent", Arg::Text("45%"))],
            )
            .unwrap(),
    )
}

/// M4 success criterion: the six Arabic forms, for 0, 1, 2, 3, 11 and 100,
/// compared as whole texts, for every message that counts (review of #43:
/// only some were checked). A new counted message must be added here: the
/// tests below fail otherwise (a `PLURAL` select missing from this table,
/// or `$count` shown without `PLURAL`).
const ARABIC: &[(&str, [&str; 6])] = &[
    (
        "import-done",
        [
            "لم يُضف أي تحميل.",
            "أُضيف تحميل واحد.",
            "أُضيف تحميلان.",
            "أُضيفت 3 تحميلات.",
            "أُضيف 11 تحميلًا.",
            "أُضيف 100 تحميل.",
        ],
    ),
    (
        "downloads-count",
        [
            "لا تحميلات",
            "تحميل واحد",
            "تحميلان",
            "3 تحميلات",
            "11 تحميلًا",
            "100 تحميل",
        ],
    ),
    (
        "connections-count",
        [
            "لا اتصالات",
            "اتصال واحد",
            "اتصالان",
            "3 اتصالات",
            "11 اتصالًا",
            "100 اتصال",
        ],
    ),
    (
        "queues-count",
        [
            "لا طوابير",
            "طابور واحد",
            "طابوران",
            "3 طوابير",
            "11 طابورًا",
            "100 طابور",
        ],
    ),
    (
        "files-count",
        [
            "لا ملفات",
            "ملف واحد",
            "ملفان",
            "3 ملفات",
            "11 ملفًا",
            "100 ملف",
        ],
    ),
    (
        "seconds-left",
        [
            "0 ثانية",
            "ثانية واحدة",
            "ثانيتان",
            "3 ثوانٍ",
            "11 ثانيةً",
            "100 ثانية",
        ],
    ),
    (
        "minutes-left",
        [
            "0 دقيقة",
            "دقيقة واحدة",
            "دقيقتان",
            "3 دقائق",
            "11 دقيقةً",
            "100 دقيقة",
        ],
    ),
    (
        "hours-left",
        [
            "0 ساعة",
            "ساعة واحدة",
            "ساعتان",
            "3 ساعات",
            "11 ساعةً",
            "100 ساعة",
        ],
    ),
    (
        "progress-label",
        [
            "45%",
            "45% في جزء واحد",
            "45% في جزأين",
            "45% في 3 أجزاء",
            "45% في 11 جزءًا",
            "45% في 100 جزء",
        ],
    ),
    (
        "footer-active",
        [
            "لا شيء يعمل الآن",
            "يعمل الآن: تحميل واحد",
            "يعمل الآن: تحميلان",
            "يعمل الآن: 3 تحميلات",
            "يعمل الآن: 11 تحميلًا",
            "يعمل الآن: 100 تحميل",
        ],
    ),
    (
        "footer-waiting",
        [
            "لا شيء في الانتظار",
            "في الانتظار: تحميل واحد",
            "في الانتظار: تحميلان",
            "في الانتظار: 3 تحميلات",
            "في الانتظار: 11 تحميلًا",
            "في الانتظار: 100 تحميل",
        ],
    ),
];

const ENGLISH: &[(&str, [&str; 2])] = &[
    ("downloads-count", ["1 download", "2 downloads"]),
    ("import-done", ["1 download added.", "2 downloads added."]),
    ("connections-count", ["1 connection", "2 connections"]),
    ("queues-count", ["1 queue", "2 queues"]),
    ("files-count", ["1 file", "2 files"]),
    ("seconds-left", ["1 second", "2 seconds"]),
    ("minutes-left", ["1 minute", "2 minutes"]),
    ("hours-left", ["1 hour", "2 hours"]),
    ("progress-label", ["45% in 1 part", "45% in 2 parts"]),
    (
        "footer-active",
        ["Downloading: 1 download", "Downloading: 2 downloads"],
    ),
    (
        "footer-waiting",
        ["Waiting: 1 download", "Waiting: 2 downloads"],
    ),
];

#[test]
fn arabic_plural_forms_for_0_1_2_3_11_and_100() {
    for (key, forms) in ARABIC {
        for (count, expected) in [0.0, 1.0, 2.0, 3.0, 11.0, 100.0].into_iter().zip(forms) {
            assert_eq!(text(Lang::Ar, key, count), *expected, "{key} {count}");
        }
    }
    // The edges of the CLDR ranges: few is 3–10, many is 11–99, and the
    // hundreds come back to the same forms (103 few, 111 many).
    assert_eq!(text(Lang::Ar, "downloads-count", 10.0), "10 تحميلات");
    assert_eq!(text(Lang::Ar, "downloads-count", 99.0), "99 تحميلًا");
    assert_eq!(text(Lang::Ar, "downloads-count", 103.0), "103 تحميلات");
    assert_eq!(text(Lang::Ar, "downloads-count", 111.0), "111 تحميلًا");
    assert_eq!(text(Lang::Ar, "downloads-count", 102.0), "102 تحميل");
}

#[test]
fn english_has_one_and_other_in_every_counted_message() {
    for (key, forms) in ENGLISH {
        for (count, expected) in [1.0, 2.0].into_iter().zip(forms) {
            assert_eq!(text(Lang::En, key, count), *expected, "{key} {count}");
        }
    }
    assert_eq!(text(Lang::En, "downloads-count", 0.0), "0 downloads");
}

/// Every select in every message, in values, attributes and nested
/// patterns: (message, variant keys) for those through `PLURAL`, and the
/// names of messages that select on anything else.
fn selects(lang: Lang) -> (Vec<(String, Vec<String>)>, Vec<String>) {
    fn walk(
        name: &str,
        pattern: &Pattern<&str>,
        out: &mut (Vec<(String, Vec<String>)>, Vec<String>),
    ) {
        for element in &pattern.elements {
            if let PatternElement::Placeable { expression } = element {
                walk_expression(name, expression, out);
            }
        }
    }
    fn walk_expression(
        name: &str,
        expression: &Expression<&str>,
        out: &mut (Vec<(String, Vec<String>)>, Vec<String>),
    ) {
        match expression {
            Expression::Select { selector, variants } => {
                let through_plural = matches!(
                    selector,
                    InlineExpression::FunctionReference { id, .. } if id.name == "PLURAL"
                );
                if through_plural {
                    let key = |v: &fluent_syntax::ast::Variant<&str>| match &v.key {
                        VariantKey::Identifier { name } => (*name).to_owned(),
                        VariantKey::NumberLiteral { value } => (*value).to_owned(),
                    };
                    let mut keys: Vec<String> = variants.iter().map(key).collect();
                    // The default last, marked, so the test sees which it is
                    // (round 5: the GUI's Fluent falls back to it).
                    if let Some(default) = variants.iter().find(|v| v.default) {
                        keys.push(format!("*{}", key(default)));
                    }
                    out.0.push((name.to_owned(), keys));
                } else {
                    out.1.push(name.to_owned());
                }
                for variant in variants {
                    walk(name, &variant.value, out);
                }
            }
            Expression::Inline(InlineExpression::Placeable { expression }) => {
                walk_expression(name, expression, out);
            }
            Expression::Inline(_) => {}
        }
    }
    let resource = parser::parse(lang.source()).unwrap();
    let mut out = (Vec::new(), Vec::new());
    for entry in resource.body {
        if let Entry::Message(m) = entry {
            if let Some(value) = &m.value {
                walk(m.id.name, value, &mut out);
            }
            for attribute in &m.attributes {
                walk(m.id.name, &attribute.value, &mut out);
            }
        }
    }
    out
}

/// Every counted message selects through `PLURAL` everywhere (values,
/// attributes, nested), has exactly the forms of its language (six in
/// Arabic, one and other in English), and is checked word for word above.
#[test]
fn every_counted_message_has_all_its_forms_and_is_checked() {
    let arabic: Vec<&str> = ARABIC.iter().map(|(k, _)| *k).collect();
    let english: Vec<&str> = ENGLISH.iter().map(|(k, _)| *k).collect();
    for (lang, forms, checked) in [
        (
            Lang::Ar,
            &["few", "many", "one", "other", "two", "zero"][..],
            &arabic,
        ),
        (Lang::En, &["one", "other"][..], &english),
    ] {
        let (plural, bare) = selects(lang);
        assert!(
            bare.is_empty(),
            "{}: selects without PLURAL in {bare:?}",
            lang.tag()
        );
        assert!(!plural.is_empty());
        for (message, mut keys) in plural {
            let default = keys.pop();
            assert_eq!(
                default.as_deref(),
                Some("*other"),
                "{}: {message}",
                lang.tag()
            );
            keys.sort();
            assert_eq!(keys, forms, "{}: {message}", lang.tag());
            assert!(
                checked.contains(&message.as_str()),
                "{}: {message} is not in the table of texts",
                lang.tag()
            );
        }
    }
}

/// A Latin file name inside an Arabic sentence is isolated, so it keeps
/// its own direction (the mixed-name rule of the M4 designs).
#[test]
fn values_are_isolated_inside_sentences() {
    let ar = Localizer::new(Lang::Ar);
    let text = ar
        .text(
            "notify-failed-body",
            &[
                ("name", Arg::Text("report (Part 2).mp4")),
                ("reason", Arg::Text("404")),
            ],
        )
        .unwrap();
    assert_eq!(
        text,
        "\u{2068}report (Part 2).mp4\u{2069}: \u{2068}404\u{2069}"
    );
    assert!(Lang::Ar.is_rtl() && !Lang::En.is_rtl());
}

/// A value cannot end its own isolation and reverse what follows
/// (review of #43, round 4).
#[test]
fn a_value_cannot_break_out_of_its_isolation() {
    let shown = Localizer::new(Lang::Ar)
        .text(
            "notify-failed-body",
            &[
                ("name", Arg::Text("a.mp4")),
                ("reason", Arg::Text("x\u{2069}\u{202E}gpj.exe")),
            ],
        )
        .unwrap();
    assert_eq!(shown, "\u{2068}a.mp4\u{2069}: \u{2068}xgpj.exe\u{2069}");
    // Every control but tab ends a paragraph or drives a terminal (round 6).
    let shown = Localizer::new(Lang::Ar)
        .text(
            "notify-failed-body",
            &[
                ("name", Arg::Text("a")),
                ("reason", Arg::Text("x\u{85}\u{1c}\n\u{b}\u{c}\u{1b}y\tz")),
            ],
        )
        .unwrap();
    assert_eq!(shown, "\u{2068}a\u{2069}: \u{2068}x      y z\u{2069}");
    let shown = Localizer::new(Lang::En)
        .text(
            "notify-failed-body",
            &[
                ("name", Arg::Text("a")),
                ("reason", Arg::Text("timed out\nretrying")),
            ],
        )
        .unwrap();
    assert_eq!(without_isolation(&shown), "a: timed out retrying");
    // Not a finite number: an error wherever it is shown (round 7).
    assert!(
        Localizer::new(Lang::En)
            .text("status-scheduled", &[("time", Arg::Number(f64::INFINITY))])
            .is_err()
    );
}

/// No string literal (`{ "…" }`) in any message: an escape such as
/// `{ "\\u2068" }` would put a direction mark or a number back into a
/// text after the checks below (review of #43, round 6). None is needed.
#[test]
fn no_message_uses_a_string_literal() {
    fn in_pattern(pattern: &Pattern<&str>) -> bool {
        pattern.elements.iter().any(|element| match element {
            PatternElement::Placeable { expression } => in_expression(expression),
            PatternElement::TextElement { .. } => false,
        })
    }
    fn in_expression(expression: &Expression<&str>) -> bool {
        match expression {
            Expression::Select { selector, variants } => {
                in_inline(selector) || variants.iter().any(|v| in_pattern(&v.value))
            }
            Expression::Inline(inline) => in_inline(inline),
        }
    }
    fn in_inline(inline: &InlineExpression<&str>) -> bool {
        match inline {
            InlineExpression::StringLiteral { .. } => true,
            InlineExpression::FunctionReference { arguments, .. }
            | InlineExpression::TermReference {
                arguments: Some(arguments),
                ..
            } => {
                arguments.positional.iter().any(in_inline)
                    || arguments.named.iter().any(|n| in_inline(&n.value))
            }
            InlineExpression::Placeable { expression } => in_expression(expression),
            _ => false,
        }
    }
    for lang in Lang::ALL {
        for entry in parser::parse(lang.source()).unwrap().body {
            if let Entry::Message(m) = entry
                && let Some(value) = &m.value
            {
                assert!(
                    !in_pattern(value),
                    "{}: {} uses a string literal",
                    lang.tag(),
                    m.id.name
                );
            }
        }
    }
}

/// No direction mark, isolation mark or control character written into
/// the translation files: the whole-text tables compare without the
/// isolation marks, so an unclosed U+2068 or a U+202E in a text would
/// pass them (review of #43, round 5).
#[test]
fn the_translation_files_hold_no_direction_or_control_characters() {
    for lang in Lang::ALL {
        for (n, line) in lang.source().lines().enumerate() {
            let bad = line.chars().find(|c| {
                matches!(
                    c,
                    '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}'
                ) || (c.is_control() && *c != '\t')
            });
            assert!(
                bad.is_none(),
                "{} line {}: U+{:04X}",
                lang.tag(),
                n + 1,
                bad.map_or(0, |c| c as u32)
            );
        }
    }
}

#[test]
fn a_missing_key_is_an_error_not_a_crash() {
    assert!(Localizer::new(Lang::Ar).text("no-such-key", &[]).is_err());
}

/// CLDR's Arabic cardinal rule, written out: what `PLURAL` must return.
fn cldr_arabic(n: u64) -> &'static str {
    match (n, n % 100) {
        (0, _) => "zero",
        (1, _) => "one",
        (2, _) => "two",
        (_, 3..=10) => "few",
        (_, 11..=99) => "many",
        _ => "other",
    }
}

/// `PLURAL` gives CLDR's category for every number up to 1200 (the rules
/// inside fluent-bundle got 103 and 111 wrong, which is why it exists).
#[test]
fn plural_follows_cldr_for_every_number_up_to_1200() {
    for n in 0..=1200u64 {
        assert_eq!(
            zajil_i18n::plural_category("ar", n as f64),
            cldr_arabic(n),
            "{n}"
        );
        let en = if n == 1 { "one" } else { "other" };
        assert_eq!(zajil_i18n::plural_category("en", n as f64), en, "{n}");
    }
    assert_eq!(zajil_i18n::plural_category("ar", 1.5), "other");
    // Huge counts keep their category, as in the GUI (review of #43).
    assert_eq!(
        zajil_i18n::plural_category("ar", 1_000_000_000_000_003.0),
        "few"
    );
    // A count given as text that is a number picks the same form.
    let ar = Localizer::new(Lang::Ar);
    let as_text = ar
        .text("downloads-count", &[("count", Arg::Text("3"))])
        .unwrap();
    assert_eq!(without_isolation(&as_text), "3 تحميلات");
    let eastern = ar
        .text("downloads-count", &[("count", Arg::Text("١١"))])
        .unwrap();
    assert_eq!(without_isolation(&eastern), "١١ تحميلًا");
    // Not counts (round 5): a decimal comma, exponents, infinity, signs,
    // and negative numbers, which would lose their sign in the one form.
    for bad in ["1,5", "1e3", "inf", "NaN", "+3", "-3", "1,00", "١٬٠٠"] {
        assert!(
            ar.text("downloads-count", &[("count", Arg::Text(bad))])
                .is_err(),
            "{bad:?}"
        );
    }
    assert!(
        ar.text("downloads-count", &[("count", Arg::Number(-1.0))])
            .is_err()
    );
    assert!(
        ar.text("downloads-count", &[("count", Arg::Number(f64::INFINITY))])
            .is_err()
    );
    assert!(
        ar.text("downloads-count", &[("count", Arg::Number(f64::NAN))])
            .is_err()
    );
    let en = Localizer::new(Lang::En);
    let zero = en
        .text("downloads-count", &[("count", Arg::Number(-0.0))])
        .unwrap();
    assert_eq!(without_isolation(&zero), "0 downloads");
    let fraction = ar
        .text("downloads-count", &[("count", Arg::Text("٣٫٥"))])
        .unwrap();
    assert_eq!(without_isolation(&fraction), "٣٫٥ تحميل");
    // Thousands separators and the Arabic decimal point (round 4).
    let separated = ar
        .text("downloads-count", &[("count", Arg::Text("1,003"))])
        .unwrap();
    assert_eq!(without_isolation(&separated), "1,003 تحميلات");
    let arabic = ar
        .text("downloads-count", &[("count", Arg::Text("١٬٠٠٣"))])
        .unwrap();
    assert_eq!(without_isolation(&arabic), "١٬٠٠٣ تحميلات");
    // A count that is not a number is an error, not the default form.
    for bad in ["", "abc", "3 files"] {
        assert!(
            ar.text("downloads-count", &[("count", Arg::Text(bad))])
                .is_err(),
            "{bad:?}"
        );
    }
    assert!(
        ar.text("downloads-count", &[("count", Arg::Number(3.0))])
            .is_ok()
    );
    // Beyond u64: 1e20 ends in 00, so "other", as in Intl.PluralRules.
    assert_eq!(zajil_i18n::plural_category("ar", 1e20), "other");
}

/// Every message with variables, and its variables, reviewed one by one
/// (review of #43, rounds 2 and 3): `$count` is a number that needs the
/// Arabic forms and goes through `PLURAL`; the others are values formatted
/// beforehand (`$size`, `$speed`…), names, or numbers that do not agree
/// with a noun. A new message with variables, or a new variable in a
/// message, fails here until someone has looked at it.
const REVIEWED_VARIABLES: &[(&str, &[&str])] = &[
    ("add-category-extension", &["category", "extension"]),
    ("add-category-site", &["category", "site"]),
    ("add-connections-hint", &["max", "min"]),
    ("categories-remove", &["value"]),
    ("add-probe-ranges", &["size"]),
    ("add-probe-single", &["size"]),
    ("add-probe-unsure", &["size"]),
    ("connections-count", &["count"]),
    ("details-part", &["number"]),
    ("details-connections-value", &["active", "total"]),
    ("details-file-map", &["size"]),
    ("details-from-to", &["folder", "site"]),
    // M5.5: a time already formatted, an HTTP status code, and a number
    // shown alone (no noun after it).
    ("queue-starts", &["time"]),
    ("st-error", &["code"]),
    ("footer-selected", &["number"]),
    // The engine's own words (English), and two sizes formatted beforehand.
    ("details-failed", &["error"]),
    ("refresh-size", &["ours", "theirs"]),
    ("downloads-count", &["count"]),
    ("import-done", &["count"]),
    ("import-kept", &["names"]),
    ("files-count", &["count"]),
    ("footer-active", &["count"]),
    ("footer-limit", &["speed"]),
    ("footer-total-speed", &["speed"]),
    ("footer-waiting", &["count"]),
    ("hours-left", &["count"]),
    ("minutes-left", &["count"]),
    ("notify-done-body", &["name"]),
    ("notify-failed-body", &["name", "reason"]),
    ("progress-label", &["count", "percent"]),
    ("queue-window", &["start", "stop"]),
    ("queues-count", &["count"]),
    ("seconds-left", &["count"]),
    ("settings-browser-failed", &["browser", "error"]),
    ("settings-browser-ok", &["browser"]),
    ("settings-connections-hint", &["default", "max"]),
    ("size-bytes", &["value"]),
    ("size-gib", &["value"]),
    ("size-kib", &["value"]),
    ("size-mib", &["value"]),
    ("size-of-total", &["done", "total"]),
    ("speed-per-second", &["size"]),
    ("status-scheduled", &["time"]),
];

/// Every message with variables that does not count, as whole texts in
/// both languages (review of #43, rounds 3 and 4): a noun added after a
/// number, words or variables swapped, or a changed English text all fail
/// here. Each variable has a value of its own, so swapping two shows;
/// Arabic numbers are 11 (a noun after them would be accusative), English
/// numbers 1 (a noun after them would be singular).
struct Whole {
    key: &'static str,
    ar: (&'static [(&'static str, &'static str)], &'static str),
    en: (&'static [(&'static str, &'static str)], &'static str),
}

const WHOLE_TEXTS: &[Whole] = &[
    Whole {
        key: "add-category-extension",
        ar: (
            &[("category", "CAT"), ("extension", "EXT")],
            "CAT (من الامتداد EXT)",
        ),
        en: (
            &[("category", "CAT"), ("extension", "EXT")],
            "CAT (from the extension EXT)",
        ),
    },
    Whole {
        key: "add-category-site",
        ar: (
            &[("category", "CAT"), ("site", "SITE")],
            "CAT (من الموقع SITE)",
        ),
        en: (
            &[("category", "CAT"), ("site", "SITE")],
            "CAT (from the site SITE)",
        ),
    },
    Whole {
        key: "add-connections-hint",
        ar: (&[("min", "11"), ("max", "32")], "من 11 إلى 32"),
        en: (&[("min", "1"), ("max", "32")], "1 to 32"),
    },
    Whole {
        key: "add-probe-ranges",
        ar: (
            &[("size", "SIZE")],
            "SIZE. السيرفر يدعم التقسيم، فيمكن التحميل بعدة اتصالات واستكماله لاحقًا.",
        ),
        en: (
            &[("size", "SIZE")],
            "SIZE. The server allows ranges, so it can download over several connections and resume later.",
        ),
    },
    Whole {
        key: "add-probe-single",
        ar: (
            &[("size", "SIZE")],
            "SIZE. السيرفر لا يدعم التقسيم، فسيُحمَّل باتصال واحد.",
        ),
        en: (
            &[("size", "SIZE")],
            "SIZE. The server does not allow ranges, so it downloads over one connection.",
        ),
    },
    Whole {
        key: "add-probe-unsure",
        ar: (
            &[("size", "SIZE")],
            "SIZE. السيرفر يدعم التقسيم، لكنه لا يعطي الحجم أو ما يُتأكَّد به أن الملف لم يتغيّر، فسيُحمَّل باتصال واحد، ويبدأ من جديد إذا انقطع.",
        ),
        en: (
            &[("size", "SIZE")],
            "SIZE. The server allows ranges but gives no size, or no way to be sure the file has not changed, so it downloads over one connection and starts over if cut.",
        ),
    },
    Whole {
        key: "details-part",
        ar: (&[("number", "11")], "الجزء 11"),
        en: (&[("number", "1")], "Part 1"),
    },
    Whole {
        key: "details-connections-value",
        ar: (&[("active", "11"), ("total", "32")], "يعمل 11 من 32"),
        en: (&[("active", "1"), ("total", "32")], "1 of 32 working"),
    },
    Whole {
        key: "details-file-map",
        ar: (&[("size", "SIZE")], "خريطة الملف: كل مربع SIZE"),
        en: (&[("size", "SIZE")], "File map: each square is SIZE"),
    },
    Whole {
        key: "categories-remove",
        ar: (&[("value", "VALUE")], "احذف VALUE"),
        en: (&[("value", "VALUE")], "Remove VALUE"),
    },
    Whole {
        key: "import-kept",
        ar: (&[("names", "NAMES")], "هذه الطوابير أبقت إعداداتها: NAMES."),
        en: (
            &[("names", "NAMES")],
            "These queues kept their own settings: NAMES.",
        ),
    },
    Whole {
        key: "details-from-to",
        ar: (
            &[("site", "SITE"), ("folder", "FOLDER")],
            "من SITE، إلى FOLDER",
        ),
        en: (
            &[("site", "SITE"), ("folder", "FOLDER")],
            "From SITE, to FOLDER",
        ),
    },
    Whole {
        key: "details-failed",
        ar: (&[("error", "ERROR")], "توقف التحميل: ERROR"),
        en: (&[("error", "ERROR")], "The download stopped: ERROR"),
    },
    Whole {
        key: "refresh-size",
        ar: (
            &[("theirs", "THEIRS"), ("ours", "OURS")],
            "الرابط الجديد لملف آخر: حجمه THEIRS، وحجم هذا التحميل OURS.",
        ),
        en: (
            &[("theirs", "THEIRS"), ("ours", "OURS")],
            "The new link is to another file: its size is THEIRS, this download's OURS.",
        ),
    },
    Whole {
        key: "queue-starts",
        ar: (&[("time", "TIME")], "يبدأ TIME"),
        en: (&[("time", "TIME")], "Starts TIME"),
    },
    Whole {
        key: "st-error",
        ar: (&[("code", "CODE")], "خطأ CODE"),
        en: (&[("code", "CODE")], "Error CODE"),
    },
    Whole {
        key: "footer-selected",
        ar: (&[("number", "NUMBER")], "التحديد: NUMBER"),
        en: (&[("number", "NUMBER")], "Selected: NUMBER"),
    },
    Whole {
        key: "footer-limit",
        ar: (&[("speed", "SPEED")], "حد السرعة: SPEED"),
        en: (&[("speed", "SPEED")], "Speed limit: SPEED"),
    },
    Whole {
        key: "footer-total-speed",
        ar: (&[("speed", "SPEED")], "السرعة الكلية SPEED"),
        en: (&[("speed", "SPEED")], "Total speed SPEED"),
    },
    Whole {
        key: "notify-done-body",
        ar: (&[("name", "NAME")], "NAME"),
        en: (&[("name", "NAME")], "NAME"),
    },
    Whole {
        key: "notify-failed-body",
        ar: (&[("name", "NAME"), ("reason", "REASON")], "NAME: REASON"),
        en: (&[("name", "NAME"), ("reason", "REASON")], "NAME: REASON"),
    },
    Whole {
        key: "settings-browser-failed",
        ar: (
            &[("browser", "Firefox"), ("error", "ERR")],
            "Firefox: لم يُسجَّل (ERR)",
        ),
        en: (
            &[("browser", "Firefox"), ("error", "ERR")],
            "Firefox: not registered (ERR)",
        ),
    },
    Whole {
        key: "settings-browser-ok",
        ar: (&[("browser", "Chrome")], "Chrome: مسجَّل"),
        en: (&[("browser", "Chrome")], "Chrome: registered"),
    },
    Whole {
        key: "queue-window",
        ar: (
            &[("start", "02:00"), ("stop", "07:00")],
            "من 02:00 إلى 07:00",
        ),
        en: (
            &[("start", "02:00"), ("stop", "07:00")],
            "from 02:00 to 07:00",
        ),
    },
    Whole {
        key: "settings-connections-hint",
        ar: (
            &[("default", "11"), ("max", "32")],
            "الافتراضي 11، والحد الأقصى 32.",
        ),
        en: (
            &[("default", "1"), ("max", "32")],
            "1 by default, at most 32.",
        ),
    },
    Whole {
        key: "size-bytes",
        ar: (&[("value", "11")], "11 بايت"),
        en: (&[("value", "1")], "1 B"),
    },
    Whole {
        key: "size-kib",
        ar: (&[("value", "11")], "11 كيلوبايت"),
        en: (&[("value", "1")], "1 KiB"),
    },
    Whole {
        key: "size-mib",
        ar: (&[("value", "11")], "11 ميجابايت"),
        en: (&[("value", "1")], "1 MiB"),
    },
    Whole {
        key: "size-gib",
        ar: (&[("value", "11")], "11 جيجابايت"),
        en: (&[("value", "1")], "1 GiB"),
    },
    Whole {
        key: "size-of-total",
        ar: (&[("done", "DONE"), ("total", "TOTAL")], "DONE من TOTAL"),
        en: (&[("done", "DONE"), ("total", "TOTAL")], "DONE of TOTAL"),
    },
    Whole {
        key: "speed-per-second",
        ar: (&[("size", "SIZE")], "SIZE/ث"),
        en: (&[("size", "SIZE")], "SIZE/s"),
    },
    Whole {
        key: "status-scheduled",
        ar: (&[("time", "02:00")], "مجدول 02:00"),
        en: (&[("time", "02:00")], "Scheduled 02:00"),
    },
];

#[test]
fn every_message_with_values_is_checked_as_a_whole_text_in_both_languages() {
    for whole in WHOLE_TEXTS {
        for (lang, (args, expected)) in [(Lang::Ar, whole.ar), (Lang::En, whole.en)] {
            let args: Vec<(&str, Arg<'_>)> = args.iter().map(|(n, v)| (*n, Arg::Text(v))).collect();
            let shown = Localizer::new(lang).text(whole.key, &args).unwrap();
            assert_eq!(
                without_isolation(&shown),
                expected,
                "{}: {}",
                lang.tag(),
                whole.key
            );
        }
    }
    // Every message with variables is in this table or, if it counts, in
    // ARABIC and ENGLISH: nothing is left to a guess about which variables
    // are numbers.
    let checked: BTreeSet<&str> = WHOLE_TEXTS
        .iter()
        .map(|w| w.key)
        .chain(ARABIC.iter().map(|(k, _)| *k))
        .collect();
    let reviewed: BTreeSet<&str> = REVIEWED_VARIABLES.iter().map(|(k, _)| *k).collect();
    assert_eq!(checked, reviewed);
}

/// A number written into a message (`11 ثانية`, `{ 3 }`) never goes
/// through PLURAL: only messages checked as whole texts may have one, and
/// none has yet (review of #43, round 4).
#[test]
fn no_message_writes_a_number_of_its_own() {
    fn fixed(pattern: &Pattern<&str>) -> bool {
        pattern.elements.iter().any(|element| match element {
            PatternElement::TextElement { value } => value.chars().any(|c| {
                c.is_ascii_digit()
                    || ('\u{0660}'..='\u{0669}').contains(&c)
                    || ('\u{06F0}'..='\u{06F9}').contains(&c)
            }),
            PatternElement::Placeable { expression } => fixed_in(expression),
        })
    }
    fn fixed_in(expression: &Expression<&str>) -> bool {
        match expression {
            Expression::Select { variants, .. } => variants.iter().any(|v| fixed(&v.value)),
            Expression::Inline(InlineExpression::NumberLiteral { .. }) => true,
            Expression::Inline(InlineExpression::Placeable { expression }) => fixed_in(expression),
            Expression::Inline(_) => false,
        }
    }
    for lang in Lang::ALL {
        for entry in parser::parse(lang.source()).unwrap().body {
            if let Entry::Message(m) = entry
                && let Some(value) = &m.value
            {
                assert!(
                    !fixed(value),
                    "{}: {} writes a number of its own",
                    lang.tag(),
                    m.id.name
                );
            }
        }
    }
}

/// `$count` anywhere outside a `PLURAL($count)` select (its variants
/// included): a number shown without its plural form.
fn count_outside_plural(pattern: &Pattern<&str>, inside: bool) -> bool {
    pattern.elements.iter().any(|element| match element {
        PatternElement::Placeable { expression } => count_in_expression(expression, inside),
        PatternElement::TextElement { .. } => false,
    })
}

fn count_in_expression(expression: &Expression<&str>, inside: bool) -> bool {
    match expression {
        Expression::Select { selector, variants } => {
            let plural = matches!(
                selector,
                InlineExpression::FunctionReference { id, arguments }
                    if id.name == "PLURAL"
                        && matches!(arguments.positional.first(),
                            Some(InlineExpression::VariableReference { id }) if id.name == "count")
            );
            variants
                .iter()
                .any(|v| count_outside_plural(&v.value, inside || plural))
        }
        Expression::Inline(InlineExpression::VariableReference { id }) => {
            id.name == "count" && !inside
        }
        Expression::Inline(InlineExpression::Placeable { expression }) => {
            count_in_expression(expression, inside)
        }
        Expression::Inline(_) => false,
    }
}

#[test]
fn every_number_with_a_noun_goes_through_plural_and_every_variable_is_reviewed() {
    for lang in Lang::ALL {
        let resource = parser::parse(lang.source()).unwrap();
        for entry in resource.body {
            if let Entry::Message(m) = entry {
                // No attributes yet: they would need their own checks
                // (round 2: attributes were never formatted).
                assert!(
                    m.attributes.is_empty(),
                    "{}: {} has attributes",
                    lang.tag(),
                    m.id.name
                );
                if let Some(value) = &m.value {
                    assert!(
                        !count_outside_plural(value, false),
                        "{}: {} shows $count outside PLURAL($count)",
                        lang.tag(),
                        m.id.name
                    );
                }
            }
        }
        for (key, vars) in messages(lang) {
            if vars.is_empty() {
                continue;
            }
            let reviewed: Vec<&str> = REVIEWED_VARIABLES
                .iter()
                .find(|(k, _)| *k == key)
                .map(|(_, v)| v.to_vec())
                .unwrap_or_default();
            let used: Vec<&str> = vars.iter().map(String::as_str).collect();
            assert_eq!(
                used,
                reviewed,
                "{}: {key} uses {used:?}; REVIEWED_VARIABLES has {reviewed:?}",
                lang.tag()
            );
        }
    }
}
