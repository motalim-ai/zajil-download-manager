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
//! - `\` before one of `[]{}*,` is that character itself. Any other `\` is
//!   refused: the URL parser would turn it into `/` (write `%5C`).
//!
//! Patterns are expanded only in the path and query, never in the host or
//! the fragment, so a pattern can never fan out to many sites; every URL
//! made is checked to have the pattern's own host and port. The number of
//! URLs is computed, with checked arithmetic, before any is made: a huge
//! range is refused at once.

use std::fmt;

use zajil_core::Url;

/// URLs a pattern may make unless the caller asks for fewer or more.
pub const DEFAULT_MAX_URLS: u64 = 1_000;
/// The most URLs a pattern may ever make.
pub const MAX_URLS: u64 = 10_000;
/// The most digits `--width` (or a zero-padded `--from`) may ask for.
pub const MAX_WIDTH: u8 = 10;
/// The most digits a zero-padded `[START-END]` may have: a `u64` has 20.
pub const MAX_PADDED_WIDTH: usize = 20;
/// The longest pattern read, in bytes.
pub const MAX_PATTERN_BYTES: usize = 8 * 1024;

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
    /// Longer than [`MAX_PATTERN_BYTES`].
    TooLong,
    /// A tab, line break or other control character, which the URL parser
    /// would silently drop.
    ControlCharacter { at: usize },
    /// Nothing between `://` and the path.
    NoHost,
    /// A `[`, `{` or `*` in the fragment (after `#`), which is never sent.
    InFragment { at: usize },
    /// A backslash that is not before one of `[]{}*,`.
    Backslash { at: usize },
    /// A zero-padded end with a start that is not padded (`[1-010]`).
    PaddedEnd {
        at: usize,
        start: String,
        end: String,
    },
    /// A zero-padded width larger than allowed.
    WidthTooLarge { at: usize, width: usize, max: usize },
    /// A URL made by the pattern with another host or port than the
    /// pattern's own.
    HostChanged { url: String },
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
            TooLong => write!(f, "the pattern is longer than {MAX_PATTERN_BYTES} bytes"),
            ControlCharacter { at } => write!(
                f,
                "character {at}: a tab, line break or other control character is not allowed"
            ),
            NoHost => write!(f, "the pattern has no host after ://"),
            InFragment { at } => write!(
                f,
                "character {at}: patterns are not allowed after `#`, which is never sent to the server"
            ),
            Backslash { at } => write!(
                f,
                "character {at}: `\\` only goes before one of [ ] {{ }} * , (write %5C for a backslash)"
            ),
            PaddedEnd { at, start, end } => {
                write!(
                    f,
                    "character {at}: the end {end} starts with a zero but the start {start} \
                     does not: write both with the same number of digits"
                )?;
                // A suggestion only for a short end: a width beyond 65535
                // would make `format!` panic (review of #113).
                if end.len() <= MAX_PADDED_WIDTH {
                    write!(f, ", such as [{start:0>width$}-{end}]", width = end.len())?;
                }
                Ok(())
            }
            WidthTooLarge { at, width, max } => write!(
                f,
                "character {at}: {width} digits with zeros on the left, more than {max}"
            ),
            HostChanged { url } => write!(
                f,
                "the pattern makes {url}, whose host is not the pattern's own"
            ),
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
                "character {at}: the start {start} starts with a zero, so every number has {}, \
                 but the end {end} has {}",
                digits(start.len()),
                digits(end.len())
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
                "--to {end} has {}, more than --width {width}",
                digits(end.len())
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

fn digits(n: usize) -> String {
    if n == 1 {
        "1 digit".to_owned()
    } else {
        format!("{n} digits")
    }
}

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
    if pattern.len() > MAX_PATTERN_BYTES {
        return Err(PatternError::TooLong);
    }
    let (parts, host) = parse(pattern, star)?;
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
    let mut seen = std::collections::HashSet::new();
    let mut index = vec![0u64; parts.len()];
    for _ in 0..total {
        let mut url = String::new();
        for (part, &i) in parts.iter().zip(&index) {
            part.push_value(i, &mut url);
        }
        let url = checked_url(&url, &host)?;
        // `{a,a}`, or `{..,b}/x` once the URL is normalised: each once.
        if seen.insert(url.clone()) {
            urls.push(url);
        }
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

/// The host and port of a URL.
type HostPort = (String, Option<u16>);

fn host_port(url: &Url) -> HostPort {
    (
        url.host_str().unwrap_or("").to_owned(),
        url.port_or_known_default(),
    )
}

/// `url` as the engine will see it, if it is a valid http or https URL
/// with the pattern's own `host` and port.
fn checked_url(url: &str, host: &HostPort) -> Result<String, PatternError> {
    match Url::parse(url) {
        Ok(parsed) if !matches!(parsed.scheme(), "http" | "https") => Err(PatternError::NotHttp),
        Ok(parsed) if host_port(&parsed) != *host => {
            Err(PatternError::HostChanged { url: shown(url) })
        }
        Ok(parsed) => Ok(parsed.into()),
        Err(err) => Err(PatternError::InvalidUrl {
            url: shown(url),
            reason: err.to_string(),
        }),
    }
}

/// `url` as an error may show it: without a user name and password, and
/// without its query and fragment, which may hold a token (owner's decision
/// on #28: tokens are never printed). It is cut as text, since it may not
/// parse.
pub(crate) fn shown(url: &str) -> String {
    let mut url = match url.find(['?', '#']) {
        Some(end) => url[..end].to_owned(),
        None => url.to_owned(),
    };
    if let Some(after_scheme) = url.find("://").map(|i| i + 3) {
        let host_end = url[after_scheme..]
            .find('/')
            .map_or(url.len(), |i| i + after_scheme);
        if let Some(at) = url[after_scheme..host_end].rfind('@') {
            url.replace_range(after_scheme..=after_scheme + at, "");
        }
    }
    url
}

fn parse(pattern: &str, star: Option<&Star>) -> Result<(Vec<Part>, HostPort), PatternError> {
    let chars: Vec<char> = pattern.chars().collect();
    // The URL parser drops tabs and line breaks without a word, which would
    // move a pattern into the host (`http://\t/[1-3].example.com`).
    if let Some(i) = chars.iter().position(|c| c.is_control()) {
        return Err(PatternError::ControlCharacter { at: i + 1 });
    }
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
    if host_end == scheme_end {
        return Err(PatternError::NoHost);
    }
    let mut in_ipv6 = false;
    for (i, &c) in chars.iter().enumerate().take(host_end).skip(scheme_end) {
        match c {
            '[' if !in_ipv6 && is_ipv6_literal(&chars[i + 1..host_end]) => in_ipv6 = true,
            ']' if in_ipv6 => in_ipv6 = false,
            '[' | ']' | '{' | '}' | '*' | '\\' => return Err(PatternError::InHost { at: i + 1 }),
            _ => {}
        }
    }

    let prefix: String = chars[..host_end].iter().collect();
    let host = Url::parse(&format!("{prefix}/"))
        .map(|url| host_port(&url))
        .map_err(|err| PatternError::InvalidUrl {
            url: shown(&prefix),
            reason: err.to_string(),
        })?;
    let mut parts = vec![Part::Text(prefix)];
    let mut in_fragment = false;
    let mut text = String::new();
    let mut star_seen = false;
    let mut i = host_end;
    while i < chars.len() {
        let c = chars[i];
        if in_fragment && matches!(c, '[' | '{' | '*') {
            return Err(PatternError::InFragment { at: i + 1 });
        }
        match c {
            '\\' if i + 1 < chars.len() && is_special(chars[i + 1]) => {
                text.push(chars[i + 1]);
                i += 2;
                continue;
            }
            '\\' => return Err(PatternError::Backslash { at: i + 1 }),
            '#' if !in_fragment => {
                in_fragment = true;
                text.push(c);
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
    Ok((parts, host))
}

fn is_special(c: char) -> bool {
    matches!(c, '[' | ']' | '{' | '}' | '*' | ',')
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
            '\\' => return Err(PatternError::Backslash { at: i + 1 }),
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
    bounds_part(start, end, step, None, MAX_PADDED_WIDTH, at).map_err(|err| match err {
        PatternError::BadRange { .. } => bad(),
        other => other,
    })
}

/// A `--from` or `--to` as an error shows it: cut to 20 characters.
fn shown_bound(text: &str) -> String {
    if text.chars().count() > MAX_PADDED_WIDTH {
        format!(
            "{}…",
            text.chars().take(MAX_PADDED_WIDTH).collect::<String>()
        )
    } else {
        text.to_owned()
    }
}

/// The `*` counter from `--from`, `--to` and `--width`; the `*` is
/// character `at`.
fn star_range(star: &Star, at: usize) -> Result<Part, PatternError> {
    // `--from` and `--to` come from outside the pattern and its length cap:
    // no number or letter needs more than a u64's 20 digits.
    if star.from.len() > MAX_PADDED_WIDTH || star.to.len() > MAX_PADDED_WIDTH {
        return Err(PatternError::BadCounter {
            from: shown_bound(&star.from),
            to: shown_bound(&star.to),
        });
    }
    if let Some(width) = star.width
        && !(1..=MAX_WIDTH).contains(&width)
    {
        return Err(PatternError::BadWidth { width });
    }
    bounds_part(
        &star.from,
        &star.to,
        1,
        star.width,
        usize::from(MAX_WIDTH),
        at,
    )
    .map_err(|err| match err {
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
    max_width: usize,
    at: usize,
) -> Result<Part, PatternError> {
    let text = format!("{start}-{end}");
    let digits = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_digit());
    let letter = |s: &str| s.len() == 1 && s.as_bytes()[0].is_ascii_alphabetic();

    if digits(start) && digits(end) {
        // curl's rule: a start of more than one digit with a leading zero
        // gives the width of every number.
        let padded = start.len() > 1 && start.starts_with('0');
        let end_padded = end.len() > 1 && end.starts_with('0');
        let mut pad = 0;
        if padded {
            if start.len() > max_width {
                return Err(PatternError::WidthTooLarge {
                    at,
                    width: start.len(),
                    max: max_width,
                });
            }
            if start.len() != end.len() {
                return Err(PatternError::WidthMismatch {
                    at,
                    start: start.to_owned(),
                    end: end.to_owned(),
                });
            }
            pad = start.len();
        } else if end_padded {
            return Err(PatternError::PaddedEnd {
                at,
                start: start.to_owned(),
                end: end.to_owned(),
            });
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
                 but the end 100 has 3 digits",
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

    /// Review of #113: the host could be reached through an empty host
    /// part or a character the URL parser drops.
    #[test]
    fn patterns_can_never_reach_the_host() {
        for (pattern, want) in [
            ("http:///[1-3].example.com/f", PatternError::NoHost),
            ("https:///{a,b}.e.com", PatternError::NoHost),
            (
                "http://\t/[1-3].example.com/f",
                PatternError::ControlCharacter { at: 8 },
            ),
            (
                "http://\r\n/[1-2].e.com/f",
                PatternError::ControlCharacter { at: 8 },
            ),
            (
                "http://h/[1-2]\n",
                PatternError::ControlCharacter { at: 15 },
            ),
        ] {
            assert_eq!(error(pattern), want, "{pattern:?}");
        }
        assert_eq!(
            PatternError::NoHost.to_string(),
            "the pattern has no host after ://"
        );
    }

    /// Every URL made keeps the pattern's own host and port.
    #[test]
    fn every_url_keeps_the_patterns_host() {
        assert_eq!(
            urls("http://h:8080/[1-2]"),
            ["http://h:8080/1", "http://h:8080/2"]
        );
        let host = (String::from("h"), Some(80));
        assert!(matches!(
            checked_url("http://other/1", &host),
            Err(PatternError::HostChanged { .. })
        ));
    }

    /// Review of #113: a huge zero-padded width made `format!` panic.
    #[test]
    fn a_huge_padded_width_is_refused_without_panicking() {
        let start = format!("{}1", "0".repeat(65_535));
        let end = format!("{}9", "0".repeat(65_535));
        assert_eq!(
            expand(&format!("http://h/[{start}-{end}]"), None, MAX_URLS).unwrap_err(),
            PatternError::TooLong
        );
        let start = format!("{}1", "0".repeat(20));
        let end = format!("{}9", "0".repeat(20));
        assert_eq!(
            error(&format!("http://h/[{start}-{end}]")),
            PatternError::WidthTooLarge {
                at: 10,
                width: 21,
                max: 20
            }
        );
        let padded = format!("{}1", "0".repeat(19));
        assert_eq!(
            urls(&format!("http://h/[{padded}-{}2]", "0".repeat(19))).len(),
            2
        );
        let big = format!("{}1", "0".repeat(65_535));
        let to = format!("{}3", "0".repeat(65_535));
        assert_eq!(
            expand("http://h/f*", Some(&star(&big, &to, None)), MAX_URLS).unwrap_err(),
            PatternError::BadCounter {
                from: format!("{}…", "0".repeat(20)),
                to: format!("{}…", "0".repeat(20)),
            }
        );
        assert_eq!(
            expand(
                "http://h/f*",
                Some(&star("00000000001", "00000000003", None)),
                MAX_URLS
            )
            .unwrap_err(),
            PatternError::WidthTooLarge {
                at: 11,
                width: 11,
                max: 10
            }
        );
        assert_eq!(
            error(&format!("http://h/{}[1-2]", "a".repeat(MAX_PATTERN_BYTES))),
            PatternError::TooLong
        );
    }

    #[test]
    fn patterns_are_refused_in_the_fragment() {
        assert_eq!(
            error("http://h/a#[1-2]"),
            PatternError::InFragment { at: 12 }
        );
        assert_eq!(
            error("http://h/[1-2]#{a,b}"),
            PatternError::InFragment { at: 16 }
        );
        assert_eq!(
            urls(r"http://h/[1-2]#x\[y"),
            ["http://h/1#x[y", "http://h/2#x[y"]
        );
    }

    /// Review of #113: a user name and password are never shown.
    #[test]
    fn errors_never_show_a_password() {
        for pattern in [
            "http://user:secretpw@h:99999/[1-2]?sig=T",
            "http://u:secretpw@/[1-2].e.com/",
        ] {
            let err = error(pattern);
            let text = err.to_string();
            assert!(!text.contains("secretpw"), "{text}");
            assert!(!text.contains("sig=T"), "{text}");
        }
        assert_eq!(shown("http://u:p@h:1/a?b#c"), "http://h:1/a");
        assert_eq!(shown("http://h/a@b"), "http://h/a@b");
    }

    #[test]
    fn a_bare_backslash_is_refused() {
        assert_eq!(
            error(r"http://h/a\b[1-2]"),
            PatternError::Backslash { at: 11 }
        );
        assert_eq!(
            error(r"http://h/[1-2]\"),
            PatternError::Backslash { at: 15 }
        );
        assert_eq!(
            error(r"http://h/{a\b,c}"),
            PatternError::Backslash { at: 12 }
        );
        assert_eq!(
            error(r"http://h/\\[1-2]"),
            PatternError::Backslash { at: 10 }
        );
    }

    #[test]
    fn a_padded_end_needs_a_padded_start() {
        let err = error("http://h/[1-010]");
        assert_eq!(
            err,
            PatternError::PaddedEnd {
                at: 10,
                start: "1".into(),
                end: "010".into()
            }
        );
        assert_eq!(
            err.to_string(),
            "character 10: the end 010 starts with a zero but the start 1 does not: write both \
             with the same number of digits, such as [001-010]"
        );
        assert!(matches!(
            expand("http://h/f*", Some(&star("1", "0100", Some(3))), MAX_URLS),
            Err(PatternError::PaddedEnd { .. })
        ));
        assert_eq!(
            error("http://h/[01-5]").to_string(),
            "character 10: the start 01 starts with a zero, so every number has 2 digits, \
             but the end 5 has 1 digit"
        );
        // `[0-100]`: a single 0 pads nothing.
        assert_eq!(urls("http://h/[0-100]").len(), 101);
    }

    #[test]
    fn the_same_url_is_made_once() {
        assert_eq!(urls("http://h/{a,a,b}"), ["http://h/a", "http://h/b"]);
        assert_eq!(urls("http://h/{..,b}/x"), ["http://h/x", "http://h/b/x"]);
    }

    /// Review of #113: a padded end too long to suggest a fix for, and a
    /// `--to` longer than any number, are shown without panicking.
    #[test]
    fn long_bounds_are_shown_without_panicking() {
        let to = format!("0{}3", "0".repeat(70_000));
        let err = expand("http://h/f*", Some(&star("1", &to, None)), MAX_URLS).unwrap_err();
        assert!(matches!(err, PatternError::BadCounter { .. }), "{err:?}");
        let text = err.to_string();
        assert!(text.len() < 300, "{}", text.len());
        let err = PatternError::PaddedEnd {
            at: 10,
            start: "1".into(),
            end: format!("0{}", "0".repeat(70_000)),
        };
        assert!(!err.to_string().contains("such as"));
    }
}
