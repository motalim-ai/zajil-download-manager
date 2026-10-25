//! Batch download patterns (ADR-0010, section 8): one URL with ranges and
//! lists that stand for many URLs, such as `img[001-100].jpg`.
//!
//! - `[START-END]` and `[START-END:STEP]`: numbers, zero-padded to the
//!   width of START when START starts with a zero; or ASCII letters of one
//!   case (`[a-z]`).
//! - `{one,two}`: a list.
//! - `*`: a counter whose range comes from outside the pattern ([`Star`]),
//!   as `--from`, `--to` and `--width` on the command line (owner's
//!   decision on ADR-0010).
//! - `\` before one of `[]{}*,\` is that character itself.
//!
//! Patterns are expanded only after the host, never in it, so a pattern can
//! never fan out to many sites. The number of URLs is computed, with checked
//! arithmetic, before any is made: a huge range is refused at once.

use std::fmt;

use zajil_core::Url;

/// URLs a pattern may make unless the caller asks for fewer or more.
pub const DEFAULT_MAX_URLS: u64 = 1_000;
/// The most URLs a pattern may ever make.
pub const MAX_URLS: u64 = 10_000;
/// The most digits `--width` may ask for.
pub const MAX_WIDTH: u8 = 10;

/// The range of the `*` counter: `--from`, `--to` and `--width`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Star {
    pub from: String,
    pub to: String,
    /// Digits of each number, padded with zeros on the left.
    pub width: Option<u8>,
}

/// Why a pattern was refused. Positions count characters of the pattern
/// from 1.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PatternError {
    /// Not an `http://` or `https://` URL.
    NotHttp,
    /// A `[`, `{` or `*` inside the host.
    InHost { at: usize },
    /// A `[` or `{` without its `]` or `}`.
    Unclosed { at: usize, open: char },
    /// A `]` or `}` that closes nothing.
    Unopened { at: usize, close: char },
    /// A pattern inside another.
    Nested { at: usize },
    /// `[]`, or a `{}` list or one of its items empty.
    Empty { at: usize },
    /// A range that is not `START-END` or `START-END:STEP`.
    BadRange { at: usize, text: String },
    /// A range that mixes digits and letters, or capitals and small letters.
    Mixed { at: usize, text: String },
    /// A range that goes down.
    Descending { at: usize, text: String },
    /// A step of zero.
    ZeroStep { at: usize },
    /// A zero-padded start and an end of another width.
    WidthMismatch {
        at: usize,
        start: String,
        end: String,
    },
    /// A number too large to count.
    TooLarge { at: usize },
    /// `--from` and `--to` that are not two numbers or two letters.
    BadCounter { from: String, to: String },
    /// A `*` without `--from` and `--to`.
    StarNeedsRange { at: usize },
    /// More than one `*`.
    SecondStar { at: usize },
    /// `--from`, `--to` or `--width` without a `*` in the pattern.
    NoStar,
    /// `--width` with letters, or outside 1 to [`MAX_WIDTH`].
    BadWidth { width: u8 },
    /// The width a zero-padded `--from` implies, and another `--width`.
    WidthConflict { implied: usize, given: u8 },
    /// An end with more digits than `--width`.
    WidthTooSmall { end: String, width: u8 },
    /// No range, list or `*`: a single URL.
    NoPattern,
    /// More URLs than allowed (`count` is `None` when it does not even fit
    /// in a number).
    TooMany { count: Option<u64>, max: u64 },
    /// A URL the pattern makes is not a valid http or https URL.
    InvalidUrl { url: String, reason: String },
}

impl fmt::Display for PatternError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        use PatternError::*;
        match self {
            NotHttp => write!(f, "the pattern must start with http:// or https://"),
            InHost { at } => write!(
                f,
                "character {at}: patterns are not allowed in the host, only after it"
            ),
            Unclosed { at, open } => {
                let close = if *open == '[' { ']' } else { '}' };
                write!(f, "character {at}: `{open}` has no closing `{close}`")
            }
            Unopened { at, close } => write!(
                f,
                "character {at}: `{close}` closes nothing (write `\\{close}` for the character itself)"
            ),
            Nested { at } => write!(f, "character {at}: a pattern cannot be inside another"),
            Empty { at } => write!(f, "character {at}: empty range or list"),
            BadRange { at, text } => write!(
                f,
                "character {at}: `[{text}]` is not a range: use [START-END] or [START-END:STEP], \
                 such as [001-100] or [a-z]"
            ),
            Mixed { at, text } => write!(
                f,
                "character {at}: `{text}` mixes digits and letters, or capital and small letters"
            ),
            Descending { at, text } => write!(
                f,
                "character {at}: `{text}` goes down: the start must not be larger than the end"
            ),
            ZeroStep { at } => write!(f, "character {at}: the step must be at least 1"),
            WidthMismatch { at, start, end } => write!(
                f,
                "character {at}: the start {start} starts with a zero, so every number has {} \
                 digits, but the end {end} has {}",
                start.len(),
                end.len()
            ),
            TooLarge { at } => write!(f, "character {at}: the number is too large"),
            BadCounter { from, to } => write!(
                f,
                "--from {from} --to {to}: use two numbers or two letters, such as \
                 --from 1 --to 100 or --from a --to z"
            ),
            StarNeedsRange { at } => write!(
                f,
                "character {at}: `*` needs --from and --to (write `\\*` for the character itself)"
            ),
            SecondStar { at } => write!(f, "character {at}: only one `*` is allowed"),
            NoStar => write!(f, "--from, --to and --width need a `*` in the pattern"),
            BadWidth { width } => write!(
                f,
                "--width {width}: the width is for numbers only, from 1 to {MAX_WIDTH}"
            ),
            WidthConflict { implied, given } => write!(
                f,
                "--from has {implied} digits with zeros on the left, but --width is {given}"
            ),
            WidthTooSmall { end, width } => write!(
                f,
                "--to {end} has {} digits, more than --width {width}",
                end.len()
            ),
            NoPattern => write!(
                f,
                "the URL has no range, list or `*`: use `zajil get` for a single file"
            ),
            TooMany {
                count: Some(count),
                max,
            } => write!(
                f,
                "the pattern makes {count} URLs, more than the limit of {max}"
            ),
            TooMany { count: None, max } => {
                write!(f, "the pattern makes more URLs than the limit of {max}")
            }
            InvalidUrl { url, reason } => {
                write!(
                    f,
                    "the pattern makes {url}, which is not a valid URL: {reason}"
                )
            }
        }
    }
}

impl std::error::Error for PatternError {}

/// A piece of a parsed pattern.
#[derive(Debug, Clone)]
enum Part {
    Text(String),
    Numbers {
        start: u64,
        step: u64,
        count: u64,
        width: usize,
    },
    Letters {
        start: u8,
        step: u8,
        count: u64,
    },
    List(Vec<String>),
}

impl Part {
    fn count(&self) -> u64 {
        match self {
            Part::Text(_) => 1,
            Part::Numbers { count, .. } | Part::Letters { count, .. } => *count,
            Part::List(items) => items.len() as u64,
        }
    }

    /// The `index`-th value (`index < count`).
    fn push_value(&self, index: u64, out: &mut String) {
        match self {
            Part::Text(text) => out.push_str(text),
            Part::Numbers {
                start, step, width, ..
            } => {
                // In range: start + (count - 1) * step <= end, checked when
                // parsed.
                let n = start + index * step;
                out.push_str(&format!("{n:0width$}"));
            }
            Part::Letters { start, step, .. } => {
                out.push(char::from(start + index as u8 * step));
            }
            Part::List(items) => out.push_str(&items[index as usize]),
        }
    }
}

/// The URLs `pattern` stands for, in order (the rightmost range changes
/// fastest), at most `max` of them (which is itself capped at
/// [`MAX_URLS`]). `star` is the range of a `*` in the pattern.
pub fn expand(pattern: &str, star: Option<&Star>, max: u64) -> Result<Vec<String>, PatternError> {
    let max = max.min(MAX_URLS);
    let parts = parse(pattern, star)?;
    if parts.iter().all(|part| matches!(part, Part::Text(_))) {
        return Err(PatternError::NoPattern);
    }
    let mut total: u64 = 1;
    for part in &parts {
        total = match total.checked_mul(part.count()) {
            Some(total) => total,
            None => return Err(PatternError::TooMany { count: None, max }),
        };
    }
    if total > max {
        return Err(PatternError::TooMany {
            count: Some(total),
            max,
        });
    }

    let mut urls = Vec::with_capacity(total as usize);
    let mut index = vec![0u64; parts.len()];
    for _ in 0..total {
        let mut url = String::new();
        for (part, &i) in parts.iter().zip(&index) {
            part.push_value(i, &mut url);
        }
        urls.push(checked_url(&url)?);
        // The odometer: the rightmost part turns first.
        for (part, i) in parts.iter().zip(index.iter_mut()).rev() {
            *i += 1;
            if *i < part.count() {
                break;
            }
            *i = 0;
        }
    }
    Ok(urls)
}

/// `url` as the engine will see it, if it is a valid http or https URL.
fn checked_url(url: &str) -> Result<String, PatternError> {
    match Url::parse(url) {
        Ok(parsed) if matches!(parsed.scheme(), "http" | "https") => Ok(parsed.into()),
        Ok(_) => Err(PatternError::NotHttp),
        Err(err) => Err(PatternError::InvalidUrl {
            url: shown(url),
            reason: err.to_string(),
        }),
    }
}

/// `url` without its query and fragment, which may hold a token (owner's
/// decision on #28: tokens are never printed).
fn shown(url: &str) -> String {
    match url.find(['?', '#']) {
        Some(end) => url[..end].to_owned(),
        None => url.to_owned(),
    }
}

fn parse(pattern: &str, star: Option<&Star>) -> Result<Vec<Part>, PatternError> {
    let chars: Vec<char> = pattern.chars().collect();
    let scheme_end = ["http://", "https://"]
        .iter()
        .find(|scheme| {
            chars.len() >= scheme.len()
                && chars[..scheme.len()]
                    .iter()
                    .collect::<String>()
                    .eq_ignore_ascii_case(scheme)
        })
        .map(|scheme| scheme.len())
        .ok_or(PatternError::NotHttp)?;
    // The host (and port, and any user name) ends at the first `/`, `?` or
    // `#`. Inside it, only an IPv6 literal may use brackets.
    let host_end = chars[scheme_end..]
        .iter()
        .position(|c| matches!(c, '/' | '?' | '#'))
        .map_or(chars.len(), |p| p + scheme_end);
    let mut in_ipv6 = false;
    for (i, &c) in chars.iter().enumerate().take(host_end).skip(scheme_end) {
        match c {
            '[' if !in_ipv6 && is_ipv6_literal(&chars[i + 1..host_end]) => in_ipv6 = true,
            ']' if in_ipv6 => in_ipv6 = false,
            '[' | ']' | '{' | '}' | '*' | '\\' => return Err(PatternError::InHost { at: i + 1 }),
            _ => {}
        }
    }

    let mut parts = vec![Part::Text(chars[..host_end].iter().collect())];
    let mut text = String::new();
    let mut star_seen = false;
    let mut i = host_end;
    while i < chars.len() {
        let c = chars[i];
        match c {
            '\\' if i + 1 < chars.len() && is_special(chars[i + 1]) => {
                text.push(chars[i + 1]);
                i += 2;
                continue;
            }
            '[' | '{' => {
                let close = if c == '[' { ']' } else { '}' };
                let end = closing(&chars, i, close)?;
                let inner: String = chars[i + 1..end].iter().collect();
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(if c == '[' {
                    range(&inner, i + 1)?
                } else {
                    list(&chars[i + 1..end], i + 1)?
                });
                i = end + 1;
                continue;
            }
            ']' | '}' => {
                return Err(PatternError::Unopened {
                    at: i + 1,
                    close: c,
                });
            }
            '*' => {
                if star_seen {
                    return Err(PatternError::SecondStar { at: i + 1 });
                }
                star_seen = true;
                let Some(star) = star else {
                    return Err(PatternError::StarNeedsRange { at: i + 1 });
                };
                if !text.is_empty() {
                    parts.push(Part::Text(std::mem::take(&mut text)));
                }
                parts.push(star_range(star, i + 1)?);
            }
            _ => text.push(c),
        }
        i += 1;
    }
    if !text.is_empty() {
        parts.push(Part::Text(text));
    }
    if star.is_some() && !star_seen {
        return Err(PatternError::NoStar);
    }
    Ok(parts)
}

fn is_special(c: char) -> bool {
    matches!(c, '[' | ']' | '{' | '}' | '*' | ',' | '\\')
}

/// Whether the host text after a `[` is an IPv6 literal (`::1]`, …): hex
/// digits, colons and dots up to a `]`, with at least one colon.
fn is_ipv6_literal(rest: &[char]) -> bool {
    let Some(end) = rest.iter().position(|&c| c == ']') else {
        return false;
    };
    let inner = &rest[..end];
    inner.contains(&':')
        && inner
            .iter()
            .all(|c| c.is_ascii_hexdigit() || matches!(c, ':' | '.'))
}

/// The index of the `close` that ends the group opened at `open`.
fn closing(chars: &[char], open: usize, close: char) -> Result<usize, PatternError> {
    let mut i = open + 1;
    while i < chars.len() {
        match chars[i] {
            '\\' if i + 1 < chars.len() && is_special(chars[i + 1]) => i += 1,
            c if c == close => return Ok(i),
            '[' | '{' | '*' => return Err(PatternError::Nested { at: i + 1 }),
            ']' | '}' => {
                return Err(PatternError::Unopened {
                    at: i + 1,
                    close: chars[i],
                });
            }
            _ => {}
        }
        i += 1;
    }
    Err(PatternError::Unclosed {
        at: open + 1,
        open: chars[open],
    })
}

/// `{a,b,c}`: `items` are the characters between the braces, which start at
/// character `at`.
fn list(items: &[char], at: usize) -> Result<Part, PatternError> {
    let mut out = Vec::new();
    let mut item = String::new();
    let mut item_at = at + 1;
    let mut i = 0;
    while i <= items.len() {
        match items.get(i) {
            Some('\\') if items.get(i + 1).is_some_and(|&c| is_special(c)) => {
                item.push(items[i + 1]);
                i += 2;
                continue;
            }
            Some(',') | None => {
                if item.is_empty() {
                    return Err(PatternError::Empty { at: item_at });
                }
                out.push(std::mem::take(&mut item));
                item_at = at + i + 2;
            }
            Some(&c) => item.push(c),
        }
        i += 1;
    }
    Ok(Part::List(out))
}

/// `[START-END]` or `[START-END:STEP]`: `inner` is the text between the
/// brackets, and the `[` is character `at`.
fn range(inner: &str, at: usize) -> Result<Part, PatternError> {
    if inner.is_empty() {
        return Err(PatternError::Empty { at });
    }
    let bad = || PatternError::BadRange {
        at,
        text: inner.to_owned(),
    };
    let (bounds, step) = match inner.split_once(':') {
        Some((bounds, step)) => (bounds, Some(step)),
        None => (inner, None),
    };
    let step = match step {
        None => 1,
        Some(step) if !step.is_empty() && step.bytes().all(|b| b.is_ascii_digit()) => step
            .parse::<u64>()
            .map_err(|_| PatternError::TooLarge { at })?,
        Some(_) => return Err(bad()),
    };
    if step == 0 {
        return Err(PatternError::ZeroStep { at });
    }
    let (start, end) = bounds.split_once('-').ok_or_else(bad)?;
    bounds_part(start, end, step, None, at).map_err(|err| match err {
        PatternError::BadRange { .. } => bad(),
        other => other,
    })
}

/// The `*` counter from `--from`, `--to` and `--width`; the `*` is
/// character `at`.
fn star_range(star: &Star, at: usize) -> Result<Part, PatternError> {
    if let Some(width) = star.width
        && !(1..=MAX_WIDTH).contains(&width)
    {
        return Err(PatternError::BadWidth { width });
    }
    bounds_part(&star.from, &star.to, 1, star.width, at).map_err(|err| match err {
        PatternError::BadRange { .. } => PatternError::BadCounter {
            from: star.from.clone(),
            to: star.to.clone(),
        },
        other => other,
    })
}

fn bounds_part(
    start: &str,
    end: &str,
    step: u64,
    width: Option<u8>,
    at: usize,
) -> Result<Part, PatternError> {
    let text = format!("{start}-{end}");
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let letter = |s: &str| s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic();

    if digits(start) && digits(end) {
        let padded = start.len() > 1 && start.starts_with('0');
        let end_padded = end.len() > 1 && end.starts_with('0');
        let mut pad = 0;
        if padded || end_padded {
            if start.len() != end.len() {
                return Err(PatternError::WidthMismatch {
                    at,
                    start: start.to_owned(),
                    end: end.to_owned(),
                });
            }
            pad = start.len();
        }
        if let Some(given) = width {
            if pad != 0 && pad != usize::from(given) {
                return Err(PatternError::WidthConflict {
                    implied: pad,
                    given,
                });
            }
            let end_digits = end.trim_start_matches('0').len().max(1);
            if end_digits > usize::from(given) {
                return Err(PatternError::WidthTooSmall {
                    end: end.to_owned(),
                    width: given,
                });
            }
            pad = usize::from(given);
        }
        let first: u64 = start.parse().map_err(|_| PatternError::TooLarge { at })?;
        let last: u64 = end.parse().map_err(|_| PatternError::TooLarge { at })?;
        if first > last {
            return Err(PatternError::Descending { at, text });
        }
        // (last - first) / step + 1 cannot overflow: last - first < u64::MAX
        // unless first = 0 and last = u64::MAX with step 1.
        let count = ((last - first) / step)
            .checked_add(1)
            .ok_or(PatternError::TooLarge { at })?;
        return Ok(Part::Numbers {
            start: first,
            step,
            count,
            width: pad,
        });
    }
    if letter(start) && letter(end) {
        let (a, b) = (start.as_bytes()[0], end.as_bytes()[0]);
        if a.is_ascii_uppercase() != b.is_ascii_uppercase() {
            return Err(PatternError::Mixed { at, text });
        }
        if let Some(width) = width {
            return Err(PatternError::BadWidth { width });
        }
        if a > b {
            return Err(PatternError::Descending { at, text });
        }
        // At most 26 letters, so a larger step gives one letter.
        let step = u8::try_from(step.min(26)).unwrap_or(26);
        let count = u64::from((b - a) / step) + 1;
        return Ok(Part::Letters {
            start: a,
            step,
            count,
        });
    }
    let kind = |s: &str| {
        if digits(s) {
            Some(true)
        } else if letter(s) {
            Some(false)
        } else {
            None
        }
    };
    match (kind(start), kind(end)) {
        (Some(x), Some(y)) if x != y => Err(PatternError::Mixed { at, text }),
        _ => Err(PatternError::BadRange { at, text }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(pattern: &str) -> Vec<String> {
        expand(pattern, None, MAX_URLS).unwrap()
    }

    fn error(pattern: &str) -> PatternError {
        expand(pattern, None, DEFAULT_MAX_URLS).unwrap_err()
    }

    fn star(from: &str, to: &str, width: Option<u8>) -> Star {
        Star {
            from: from.into(),
            to: to.into(),
            width,
        }
    }

    /// ROADMAP M7: `img[001-100].jpg` makes exactly 100 URLs.
    #[test]
    fn img_001_to_100_makes_exactly_100_urls() {
        let made = urls("https://example.com/img[001-100].jpg");
        assert_eq!(made.len(), 100);
        assert_eq!(made[0], "https://example.com/img001.jpg");
        assert_eq!(made[9], "https://example.com/img010.jpg");
        assert_eq!(made[99], "https://example.com/img100.jpg");
        let unique: std::collections::BTreeSet<_> = made.iter().collect();
        assert_eq!(unique.len(), 100);
    }

    #[test]
    fn ranges_lists_and_steps() {
        assert_eq!(
            urls("http://h/[1-3].bin"),
            ["http://h/1.bin", "http://h/2.bin", "http://h/3.bin"]
        );
        assert_eq!(
            urls("http://h/[0-100:25]"),
            [
                "http://h/0",
                "http://h/25",
                "http://h/50",
                "http://h/75",
                "http://h/100"
            ]
        );
        assert_eq!(
            urls("http://h/[0-9:4]"),
            ["http://h/0", "http://h/4", "http://h/8"]
        );
        assert_eq!(
            urls("http://h/p[a-c]"),
            ["http://h/pa", "http://h/pb", "http://h/pc"]
        );
        assert_eq!(
            urls("http://h/[X-Z]"),
            ["http://h/X", "http://h/Y", "http://h/Z"]
        );
        assert_eq!(urls("http://h/[a-z:13]"), ["http://h/a", "http://h/n"]);
        assert_eq!(
            urls("http://h/{jan,feb}.pdf"),
            ["http://h/jan.pdf", "http://h/feb.pdf"]
        );
        assert_eq!(urls("http://h/[5-5]"), ["http://h/5"]);
    }

    /// The rightmost range changes fastest; all combinations.
    #[test]
    fn several_patterns_make_every_combination() {
        let made = urls("http://h/s[1-2]e[01-10].mkv");
        assert_eq!(made.len(), 20);
        assert_eq!(made[0], "http://h/s1e01.mkv");
        assert_eq!(made[1], "http://h/s1e02.mkv");
        assert_eq!(made[10], "http://h/s2e01.mkv");
        assert_eq!(made[19], "http://h/s2e10.mkv");
    }

    #[test]
    fn patterns_work_in_the_query_and_keep_the_rest() {
        assert_eq!(
            urls("https://h:8443/get?id=[8-9]&x={a,b}"),
            [
                "https://h:8443/get?id=8&x=a",
                "https://h:8443/get?id=8&x=b",
                "https://h:8443/get?id=9&x=a",
                "https://h:8443/get?id=9&x=b",
            ]
        );
    }

    #[test]
    fn a_backslash_makes_the_character_itself() {
        assert_eq!(
            urls(r"http://h/\[x\]{a\,b,c}"),
            ["http://h/[x]a,b", "http://h/[x]c"]
        );
    }

    #[test]
    fn an_ipv6_host_is_not_a_pattern() {
        assert_eq!(
            urls("http://[::1]:8080/f[1-2]"),
            ["http://[::1]:8080/f1", "http://[::1]:8080/f2"]
        );
    }

    /// Arabic in the path: positions count characters, not bytes.
    #[test]
    fn positions_count_characters() {
        let err = error("http://h/ملف[1-3.bin");
        assert_eq!(err, PatternError::Unclosed { at: 13, open: '[' });
    }

    /// Every wrong pattern in ADR-0010's table, with its message.
    #[test]
    fn wrong_patterns_are_refused_with_a_clear_message() {
        let cases: &[(&str, PatternError, &str)] = &[
            (
                "http://h/img[001-100.jpg",
                PatternError::Unclosed { at: 13, open: '[' },
                "character 13: `[` has no closing `]`",
            ),
            (
                "http://h/img[].jpg",
                PatternError::Empty { at: 13 },
                "character 13: empty range or list",
            ),
            (
                "http://h/img[100-1].jpg",
                PatternError::Descending {
                    at: 13,
                    text: "100-1".into(),
                },
                "character 13: `100-1` goes down: the start must not be larger than the end",
            ),
            (
                "http://h/img[01-100].jpg",
                PatternError::WidthMismatch {
                    at: 13,
                    start: "01".into(),
                    end: "100".into(),
                },
                "character 13: the start 01 starts with a zero, so every number has 2 digits, \
                 but the end 100 has 3",
            ),
            (
                "http://h/img[a-9].jpg",
                PatternError::Mixed {
                    at: 13,
                    text: "a-9".into(),
                },
                "character 13: `a-9` mixes digits and letters, or capital and small letters",
            ),
            (
                "http://h/img[a-Z].jpg",
                PatternError::Mixed {
                    at: 13,
                    text: "a-Z".into(),
                },
                "character 13: `a-Z` mixes digits and letters, or capital and small letters",
            ),
            (
                "http://h/img[1-10:0].jpg",
                PatternError::ZeroStep { at: 13 },
                "character 13: the step must be at least 1",
            ),
            (
                "http://h/img[[1-2]].jpg",
                PatternError::Nested { at: 14 },
                "character 14: a pattern cannot be inside another",
            ),
            (
                "http://h/{a,{b}}",
                PatternError::Nested { at: 13 },
                "character 13: a pattern cannot be inside another",
            ),
            (
                "http://h/img[1-20000].jpg",
                PatternError::TooMany {
                    count: Some(20_000),
                    max: 1_000,
                },
                "the pattern makes 20000 URLs, more than the limit of 1000",
            ),
            (
                "http://h/img.jpg",
                PatternError::NoPattern,
                "the URL has no range, list or `*`: use `zajil get` for a single file",
            ),
            (
                "http://h/img*.jpg",
                PatternError::StarNeedsRange { at: 13 },
                "character 13: `*` needs --from and --to (write `\\*` for the character itself)",
            ),
            (
                "http://h/img].jpg",
                PatternError::Unopened { at: 13, close: ']' },
                "character 13: `]` closes nothing (write `\\]` for the character itself)",
            ),
            (
                "http://h/[1-2-3]",
                PatternError::BadRange {
                    at: 10,
                    text: "1-2-3".into(),
                },
                "character 10: `[1-2-3]` is not a range: use [START-END] or [START-END:STEP], \
                 such as [001-100] or [a-z]",
            ),
            (
                "http://h/[1:2]",
                PatternError::BadRange {
                    at: 10,
                    text: "1:2".into(),
                },
                "character 10: `[1:2]` is not a range: use [START-END] or [START-END:STEP], \
                 such as [001-100] or [a-z]",
            ),
            (
                "http://h/[ab-c]",
                PatternError::BadRange {
                    at: 10,
                    text: "ab-c".into(),
                },
                "character 10: `[ab-c]` is not a range: use [START-END] or [START-END:STEP], \
                 such as [001-100] or [a-z]",
            ),
            (
                "http://h/{a,,b}",
                PatternError::Empty { at: 13 },
                "character 13: empty range or list",
            ),
            (
                "http://h/{}",
                PatternError::Empty { at: 11 },
                "character 11: empty range or list",
            ),
            (
                "http://[1-3].example.com/f",
                PatternError::InHost { at: 8 },
                "character 8: patterns are not allowed in the host, only after it",
            ),
            (
                "http://{a,b}.example.com/f",
                PatternError::InHost { at: 8 },
                "character 8: patterns are not allowed in the host, only after it",
            ),
            (
                "http://h*.example.com/f",
                PatternError::InHost { at: 9 },
                "character 9: patterns are not allowed in the host, only after it",
            ),
            (
                "ftp://h/[1-2]",
                PatternError::NotHttp,
                "the pattern must start with http:// or https://",
            ),
            (
                "img[1-2].jpg",
                PatternError::NotHttp,
                "the pattern must start with http:// or https://",
            ),
        ];
        for (pattern, want, message) in cases {
            let got = error(pattern);
            assert_eq!(&got, want, "{pattern}");
            assert_eq!(got.to_string(), *message, "{pattern}");
        }
    }

    /// A range too large to count, or whose product overflows, is refused
    /// before any URL is made (no memory is filled).
    #[test]
    fn huge_patterns_are_refused_before_anything_is_made() {
        assert_eq!(
            error("http://h/[0-99999999999]"),
            PatternError::TooMany {
                count: Some(100_000_000_000),
                max: 1_000
            }
        );
        assert_eq!(
            error("http://h/[0-18446744073709551615]"),
            PatternError::TooLarge { at: 10 }
        );
        assert_eq!(
            error("http://h/[0-99999999999999999999]"),
            PatternError::TooLarge { at: 10 }
        );
        assert_eq!(
            error("http://h/[1-4294967296][1-4294967296][1-4294967296]"),
            PatternError::TooMany {
                count: None,
                max: 1_000
            }
        );
        assert_eq!(
            error("http://h/[1-2:99999999999999999999]"),
            PatternError::TooLarge { at: 10 }
        );
    }

    #[test]
    fn the_limit_is_the_callers_up_to_the_hard_cap() {
        assert_eq!(expand("http://h/[1-50]", None, 50).unwrap().len(), 50);
        assert_eq!(
            expand("http://h/[1-51]", None, 50).unwrap_err(),
            PatternError::TooMany {
                count: Some(51),
                max: 50
            }
        );
        assert_eq!(
            expand("http://h/[1-10000]", None, u64::MAX).unwrap().len(),
            10_000
        );
        assert_eq!(
            expand("http://h/[1-10001]", None, u64::MAX).unwrap_err(),
            PatternError::TooMany {
                count: Some(10_001),
                max: MAX_URLS
            }
        );
    }

    #[test]
    fn a_url_that_does_not_parse_is_refused_without_its_query() {
        let err = error("http://h:99999/[1-2]?sig=SECRET");
        assert!(matches!(err, PatternError::InvalidUrl { .. }), "{err:?}");
        assert!(!err.to_string().contains("SECRET"), "{err}");
    }

    /// Owner's decision on ADR-0010: `*` is a counter, and `--width 3`
    /// gives 001.
    #[test]
    fn the_star_counter_with_width_matches_the_bracket_range() {
        let counted = expand(
            "https://example.com/img*.jpg",
            Some(&star("1", "100", Some(3))),
            MAX_URLS,
        )
        .unwrap();
        assert_eq!(counted, urls("https://example.com/img[001-100].jpg"));
        assert_eq!(
            expand("http://h/f*", Some(&star("8", "10", None)), MAX_URLS).unwrap(),
            ["http://h/f8", "http://h/f9", "http://h/f10"]
        );
        assert_eq!(
            expand("http://h/f*", Some(&star("8", "10", Some(4))), MAX_URLS).unwrap(),
            ["http://h/f0008", "http://h/f0009", "http://h/f0010"]
        );
        assert_eq!(
            expand("http://h/f*", Some(&star("08", "10", Some(2))), MAX_URLS).unwrap(),
            ["http://h/f08", "http://h/f09", "http://h/f10"]
        );
        assert_eq!(
            expand("http://h/part-*.rar", Some(&star("a", "c", None)), MAX_URLS).unwrap(),
            [
                "http://h/part-a.rar",
                "http://h/part-b.rar",
                "http://h/part-c.rar"
            ]
        );
        assert_eq!(
            expand(r"http://h/a\*b*", Some(&star("1", "2", None)), MAX_URLS).unwrap(),
            ["http://h/a*b1", "http://h/a*b2"]
        );
    }

    #[test]
    fn wrong_star_counters_are_refused_with_a_clear_message() {
        let cases: &[(&str, Star, PatternError, &str)] = &[
            (
                "http://h/a*b*",
                star("1", "2", None),
                PatternError::SecondStar { at: 13 },
                "character 13: only one `*` is allowed",
            ),
            (
                "http://h/f*",
                star("1", "100", Some(2)),
                PatternError::WidthTooSmall {
                    end: "100".into(),
                    width: 2,
                },
                "--to 100 has 3 digits, more than --width 2",
            ),
            (
                "http://h/f*",
                star("a", "z", Some(2)),
                PatternError::BadWidth { width: 2 },
                "--width 2: the width is for numbers only, from 1 to 10",
            ),
            (
                "http://h/f*",
                star("1", "9", Some(0)),
                PatternError::BadWidth { width: 0 },
                "--width 0: the width is for numbers only, from 1 to 10",
            ),
            (
                "http://h/f*",
                star("1", "9", Some(11)),
                PatternError::BadWidth { width: 11 },
                "--width 11: the width is for numbers only, from 1 to 10",
            ),
            (
                "http://h/f*",
                star("a", "9", None),
                PatternError::Mixed {
                    at: 11,
                    text: "a-9".into(),
                },
                "character 11: `a-9` mixes digits and letters, or capital and small letters",
            ),
            (
                "http://h/f*",
                star("001", "100", Some(4)),
                PatternError::WidthConflict {
                    implied: 3,
                    given: 4,
                },
                "--from has 3 digits with zeros on the left, but --width is 4",
            ),
            (
                "http://h/f*",
                star("9", "1", None),
                PatternError::Descending {
                    at: 11,
                    text: "9-1".into(),
                },
                "character 11: `9-1` goes down: the start must not be larger than the end",
            ),
            (
                "http://h/f[1-2]",
                star("1", "2", None),
                PatternError::NoStar,
                "--from, --to and --width need a `*` in the pattern",
            ),
            (
                "http://h/f*",
                star("x1", "9", None),
                PatternError::BadCounter {
                    from: "x1".into(),
                    to: "9".into(),
                },
                "--from x1 --to 9: use two numbers or two letters, such as --from 1 --to 100 \
                 or --from a --to z",
            ),
        ];
        for (pattern, counter, want, message) in cases {
            let got = expand(pattern, Some(counter), DEFAULT_MAX_URLS).unwrap_err();
            assert_eq!(&got, want, "{pattern} {counter:?}");
            assert_eq!(got.to_string(), *message, "{pattern} {counter:?}");
        }
    }
}
