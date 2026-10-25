//! robots.txt, as RFC 9309 defines it (ADR-0010 section 3).
//!
//! Only parsing and matching live here; fetching it (and what each status
//! means) is the crawler's.

use std::time::Duration;

/// Bytes of a robots.txt that are read; the rest is ignored (RFC 9309
/// §2.5: at least 500 KiB).
pub const MAX_ROBOTS_BYTES: usize = 500 * 1024;
/// The longest `Crawl-delay` honoured (ADR-0010).
pub const MAX_CRAWL_DELAY: Duration = Duration::from_secs(60);
/// Our product token: the `User-agent` group meant for Zajil.
pub const PRODUCT_TOKEN: &str = "zajil";

/// The rules of one robots.txt for Zajil.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Robots {
    rules: Vec<Rule>,
    crawl_delay: Option<Duration>,
}

#[derive(Debug, Clone, PartialEq)]
struct Rule {
    allow: bool,
    /// Normalised as [`normalise`] does, with `*` and a final `$` kept.
    path: String,
}

impl Robots {
    /// Rules that allow everything (RFC 9309 §2.3.1.3: robots.txt
    /// unavailable, such as a 404).
    pub fn allow_all() -> Self {
        Robots::default()
    }

    /// Rules that allow nothing (RFC 9309 §2.3.1.4: robots.txt
    /// unreachable, such as a 500 or a network error).
    pub fn disallow_all() -> Self {
        Robots {
            rules: vec![Rule {
                allow: false,
                path: "/".to_owned(),
            }],
            crawl_delay: None,
        }
    }

    /// The rules in `body` for [`PRODUCT_TOKEN`]: its own groups if any,
    /// else the `*` groups, else none. Only the first
    /// [`MAX_ROBOTS_BYTES`] are read.
    pub fn parse(body: &[u8]) -> Self {
        let body = &body[..body.len().min(MAX_ROBOTS_BYTES)];
        let text = String::from_utf8_lossy(body);
        let text = text.strip_prefix('\u{feff}').unwrap_or(&text);

        let mut ours = Robots::default();
        let mut star = Robots::default();
        let (mut have_ours, mut have_star) = (false, false);
        // The agents of the group being read, and whether a rule has ended
        // its list of agents (the next `User-agent` starts a new group).
        let (mut for_us, mut for_star, mut in_rules) = (false, false, false);
        for line in text.split(['\n', '\r']) {
            let line = line.split('#').next().unwrap_or("");
            let Some((key, value)) = line.split_once(':') else {
                continue;
            };
            let (key, value) = (key.trim().to_ascii_lowercase(), value.trim());
            match key.as_str() {
                "user-agent" => {
                    if in_rules {
                        (for_us, for_star, in_rules) = (false, false, false);
                    }
                    // The product token is the value up to any space or `/`
                    // (`Zajil/1.0`), compared without case.
                    let token = value
                        .split(|c: char| c.is_whitespace() || c == '/')
                        .next()
                        .unwrap_or("");
                    if token.eq_ignore_ascii_case(PRODUCT_TOKEN) {
                        for_us = true;
                        have_ours = true;
                    } else if token == "*" {
                        for_star = true;
                        have_star = true;
                    }
                }
                "allow" | "disallow" | "crawl-delay" => {
                    in_rules = true;
                    for (wanted, robots) in [(for_us, &mut ours), (for_star, &mut star)] {
                        if wanted {
                            robots.add(&key, value);
                        }
                    }
                }
                _ => {}
            }
        }
        if have_ours {
            ours
        } else if have_star {
            star
        } else {
            Robots::default()
        }
    }

    fn add(&mut self, key: &str, value: &str) {
        match key {
            "crawl-delay" => {
                if let Ok(seconds) = value.parse::<f64>()
                    && seconds.is_finite()
                    && seconds > 0.0
                {
                    let delay = Duration::from_secs_f64(seconds.min(60.0));
                    self.crawl_delay = Some(self.crawl_delay.map_or(delay, |d| d.max(delay)));
                }
            }
            // An empty `Disallow:` (or `Allow:`) is no rule at all.
            _ if value.is_empty() => {}
            _ => self.rules.push(Rule {
                allow: key == "allow",
                path: normalise(value),
            }),
        }
    }

    /// Whether `path` (the URL's path and query, such as `/a/b?c=d`) may be
    /// requested. `/robots.txt` itself always may.
    pub fn allows(&self, path: &str) -> bool {
        let path = normalise(path);
        if path == "/robots.txt" {
            return true;
        }
        // The longest matching rule wins; `Allow` wins a tie (RFC 9309
        // §2.2.2).
        let mut best: Option<(usize, bool)> = None;
        for rule in &self.rules {
            if matches(&rule.path, &path) {
                let length = rule.path.len();
                best = match best {
                    Some((l, allow)) if l > length || (l == length && allow) => Some((l, allow)),
                    _ => Some((length, rule.allow)),
                };
            }
        }
        best.is_none_or(|(_, allow)| allow)
    }

    /// The `Crawl-delay` of the group, at most [`MAX_CRAWL_DELAY`].
    pub fn crawl_delay(&self) -> Option<Duration> {
        self.crawl_delay
    }
}

/// `path` with the same percent-encoding as every other: characters that
/// need no encoding are decoded (`%7E` → `~`), the others are encoded with
/// capital hex digits (`%c3%a9` and `é` → `%C3%A9`). `*` and `$` stay, as
/// they may be wildcards.
fn normalise(path: &str) -> String {
    let bytes = path.as_bytes();
    let mut out = String::with_capacity(path.len());
    let mut i = 0;
    while i < bytes.len() {
        let mut byte = bytes[i];
        let mut escaped = false;
        if byte == b'%'
            && let (Some(h), Some(l)) = (
                bytes.get(i + 1).and_then(|b| hex(*b)),
                bytes.get(i + 2).and_then(|b| hex(*b)),
            )
        {
            byte = h << 4 | l;
            escaped = true;
            i += 2;
        }
        let plain = byte.is_ascii_alphanumeric()
            || b"-._~!&'()+,;=:@/?".contains(&byte)
            || (!escaped && matches!(byte, b'*' | b'$' | b'%'));
        if plain {
            out.push(char::from(byte));
        } else {
            out.push_str(&format!("%{byte:02X}"));
        }
        i += 1;
    }
    out
}

fn hex(b: u8) -> Option<u8> {
    char::from(b).to_digit(16).map(|d| d as u8)
}

/// Whether the rule `pattern` matches `path` from its start: `*` is any
/// run of characters, and a final `$` ends the match at the end of the
/// path; otherwise the pattern is a prefix.
fn matches(pattern: &str, path: &str) -> bool {
    let (pattern, anchored) = match pattern.strip_suffix('$') {
        Some(p) => (p, true),
        None => (pattern, false),
    };
    let (p, s) = (pattern.as_bytes(), path.as_bytes());
    // Greedy wildcard matching with backtracking to the last `*`: linear
    // in practice, O(len(p) * len(s)) at worst.
    let (mut i, mut j) = (0, 0);
    let mut star: Option<(usize, usize)> = None;
    loop {
        if i == p.len() {
            if !anchored || j == s.len() {
                return true;
            }
        } else if p[i] == b'*' {
            star = Some((i, j));
            i += 1;
            continue;
        } else if j < s.len() && p[i] == s[j] {
            i += 1;
            j += 1;
            continue;
        }
        match star {
            Some((si, sj)) if sj < s.len() => {
                star = Some((si, sj + 1));
                i = si + 1;
                j = sj + 1;
            }
            _ => return false,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn robots(text: &str) -> Robots {
        Robots::parse(text.as_bytes())
    }

    #[test]
    fn our_group_wins_over_the_default_group() {
        let r = robots("User-agent: *\nDisallow: /\n\nUser-agent: Zajil\nDisallow: /private/\n");
        assert!(r.allows("/public/a.zip"));
        assert!(!r.allows("/private/a.zip"));
    }

    #[test]
    fn the_default_group_applies_without_ours() {
        let r = robots("User-agent: otherbot\nDisallow: /\n\nUser-agent: *\nDisallow: /tmp\n");
        assert!(r.allows("/a"));
        assert!(!r.allows("/tmp/x"));
        assert!(!r.allows("/tmpfile"));
    }

    #[test]
    fn no_group_for_us_or_star_allows_everything() {
        assert!(robots("User-agent: otherbot\nDisallow: /\n").allows("/a"));
        assert!(robots("").allows("/a"));
        assert!(
            robots("Disallow: /\n").allows("/a"),
            "a rule before any agent"
        );
    }

    /// RFC 9309 §2.2.1: several agents in one group, and groups for the
    /// same agent are merged.
    #[test]
    fn groups_share_agents_and_merge() {
        let r = robots(
            "User-agent: a\nUser-agent: zajil/2.0\nDisallow: /x\n\n\
             User-agent: b\nDisallow: /y\n\n\
             User-agent: ZAJIL\nDisallow: /z\n",
        );
        assert!(!r.allows("/x"));
        assert!(r.allows("/y"));
        assert!(!r.allows("/z"));
    }

    /// RFC 9309 §2.2.2: the longest match wins; Allow wins a tie.
    #[test]
    fn the_longest_rule_wins_and_allow_wins_a_tie() {
        let r = robots(
            "User-agent: *\nDisallow: /folder/\nAllow: /folder/page\n\
             Allow: /same\nDisallow: /same\n",
        );
        assert!(!r.allows("/folder/other"));
        assert!(r.allows("/folder/page.html"));
        assert!(r.allows("/same/thing"));
        let r = robots("User-agent: *\nAllow: /a\nDisallow: /a/b\n");
        assert!(!r.allows("/a/b/c"));
        assert!(r.allows("/a/c"));
    }

    /// RFC 9309 §2.2.3: `*` and `$`.
    #[test]
    fn wildcards_and_end_anchors() {
        let r = robots("User-agent: *\nDisallow: /*.pdf$\nDisallow: /*/secret*\n");
        assert!(!r.allows("/a/b.pdf"));
        assert!(r.allows("/a/b.pdf?x=1"));
        assert!(r.allows("/a/b.pdfx"));
        assert!(!r.allows("/x/secret-plans"));
        assert!(r.allows("/secret"));
        let r = robots("User-agent: *\nDisallow: /$\n");
        assert!(!r.allows("/"));
        assert!(r.allows("/a"));
    }

    #[test]
    fn the_query_is_part_of_the_path() {
        let r = robots("User-agent: *\nDisallow: /search?q=\n");
        assert!(!r.allows("/search?q=zip"));
        assert!(r.allows("/search"));
    }

    /// RFC 9309 §2.2.2: percent-encoding is compared as the same bytes.
    #[test]
    fn percent_encoding_is_normalised() {
        let r = robots("User-agent: *\nDisallow: /caf%c3%a9\nDisallow: /%7Euser\n");
        assert!(!r.allows("/caf%C3%A9/menu"));
        assert!(!r.allows("/café/menu"));
        assert!(!r.allows("/~user/files"));
        let r = robots("User-agent: *\nDisallow: /ملفات/\n");
        assert!(!r.allows("/%D9%85%D9%84%D9%81%D8%A7%D8%AA/a.zip"));
    }

    #[test]
    fn an_empty_disallow_is_no_rule() {
        assert!(robots("User-agent: *\nDisallow:\n").allows("/anything"));
    }

    #[test]
    fn comments_case_line_ends_and_bom_are_handled() {
        let r = robots("\u{feff}USER-AGENT: *   # all\r\nDISALLOW: /a # why\rdisallow:/b\n");
        assert!(!r.allows("/a"));
        assert!(!r.allows("/b"));
        assert!(r.allows("/c"));
    }

    #[test]
    fn robots_txt_itself_is_always_allowed() {
        assert!(Robots::disallow_all().allows("/robots.txt"));
        assert!(!Robots::disallow_all().allows("/"));
        assert!(Robots::allow_all().allows("/"));
    }

    #[test]
    fn crawl_delay_is_read_and_capped() {
        assert_eq!(
            robots("User-agent: *\nCrawl-delay: 2.5\n").crawl_delay(),
            Some(Duration::from_millis(2500))
        );
        assert_eq!(
            robots("User-agent: *\nCrawl-delay: 3600\n").crawl_delay(),
            Some(MAX_CRAWL_DELAY)
        );
        for bad in ["-1", "0", "NaN", "inf", "soon"] {
            assert_eq!(
                robots(&format!("User-agent: *\nCrawl-delay: {bad}\n")).crawl_delay(),
                None,
                "{bad}"
            );
        }
    }

    /// RFC 9309 §2.5: only the first 500 KiB are read.
    #[test]
    fn only_the_first_500_kib_are_read() {
        let mut text = String::from("User-agent: *\nDisallow: /early\n");
        text.push_str(&"#".repeat(MAX_ROBOTS_BYTES));
        text.push_str("\nDisallow: /late\n");
        let r = robots(&text);
        assert!(!r.allows("/early"));
        assert!(r.allows("/late"));
    }

    /// Many wildcards against a long path do not take exponential time.
    #[test]
    fn many_wildcards_stay_fast() {
        let pattern = format!("/{}b", "*a".repeat(50));
        let r = robots(&format!("User-agent: *\nDisallow: {pattern}\n"));
        let path = format!("/{}", "a".repeat(4000));
        let started = std::time::Instant::now();
        assert!(r.allows(&path));
        assert!(started.elapsed() < Duration::from_secs(2));
    }
}
