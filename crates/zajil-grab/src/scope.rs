//! Which URLs a crawl may request (ADR-0010 sections 4 and 11).

use std::collections::HashMap;
use std::fmt;

use zajil_core::Url;

use crate::local::is_local;

/// Path segments a URL may have before it is taken for a trap.
pub const MAX_SEGMENTS: usize = 32;
/// Times one segment may appear in a path (`/a/b/a/b/a/b/a/b`).
pub const MAX_REPEATS: usize = 3;

/// A host the user allowed with `--allow-host HOST[:PORT]`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AllowedHost {
    /// Lowercase, as the URL parser writes it (`[::1]` for IPv6).
    host: String,
    /// `None`: the default ports of http and https.
    port: Option<u16>,
}

impl std::str::FromStr for AllowedHost {
    type Err = String;

    fn from_str(text: &str) -> Result<Self, String> {
        let bad = || format!("{text:?} is not HOST or HOST:PORT");
        if text.is_empty() || text.contains(['/', '?', '#', '@']) {
            return Err(bad());
        }
        // Let the URL parser read it, so the host is written as it is in
        // every link (lowercase, IPv4 in dotted form, IPv6 in brackets).
        let url = Url::parse(&format!("http://{text}/")).map_err(|_| bad())?;
        let host = url.host_str().ok_or_else(bad)?.to_owned();
        let explicit_port = text.rsplit_once(':').is_some_and(|(before, port)| {
            !before.is_empty() && !before.ends_with(':') && port.bytes().all(|b| b.is_ascii_digit())
        });
        let port = if explicit_port {
            // `url.port()` hides 80; read it from the text.
            let digits = text.rsplit_once(':').map(|(_, p)| p).unwrap_or("");
            Some(digits.parse().map_err(|_| bad())?)
        } else {
            None
        };
        Ok(AllowedHost { host, port })
    }
}

impl AllowedHost {
    fn matches(&self, url: &Url) -> bool {
        url.host_str() == Some(self.host.as_str())
            && match self.port {
                Some(port) => url.port_or_known_default() == Some(port),
                None => url.port().is_none(),
            }
    }
}

/// Why a URL is not requested.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Refusal {
    /// Another origin, not allowed.
    OtherSite,
    /// A local or private address outside the site.
    LocalAddress,
    /// A redirect from https to http.
    Downgrade,
    /// Too many path segments, or one repeated too often.
    PathTrap,
    /// A user name or password in the URL: a crawl never sends one (a
    /// redirect could carry them, review of #195).
    Credentials,
    /// Not http or https (review of #195: the engine refuses the others
    /// too, but the scope says so first).
    Scheme,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Refusal::OtherSite => "outside the site",
            Refusal::LocalAddress => "a local address",
            Refusal::Downgrade => "a redirect from https to http",
            Refusal::PathTrap => "a path that repeats itself",
            Refusal::Credentials => "a user name or password in the link",
            Refusal::Scheme => "not an http or https link",
        })
    }
}

/// Where a URL that may be requested is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Place {
    /// The site, or a host the user allowed: pages and files.
    Site,
    /// Another origin, for a file only (`--files-anywhere`).
    FileElsewhere,
}

/// What a crawl may request.
#[derive(Debug, Clone)]
pub struct Scope {
    start: Url,
    allowed: Vec<AllowedHost>,
    files_anywhere: bool,
    /// Origins the tests treat as public (two test servers are both on
    /// 127.0.0.1). Never set by the command line or the app.
    #[cfg(feature = "test-support")]
    public: Vec<String>,
}

/// The URL's origin as text (`https://host:port`).
fn origin(url: &Url) -> String {
    url.origin().ascii_serialization()
}

impl Scope {
    /// The scope of a crawl that starts at `start`. A start URL with a user
    /// name or password is refused: a crawl never sends credentials.
    pub fn new(
        start: &Url,
        allowed: Vec<AllowedHost>,
        files_anywhere: bool,
    ) -> Result<Self, String> {
        if !start.username().is_empty() || start.password().is_some() {
            return Err(
                "a crawl never sends a user name or password: remove them from the URL".to_owned(),
            );
        }
        if !matches!(start.scheme(), "http" | "https") || start.host_str().is_none() {
            return Err("the start must be an http or https URL".to_owned());
        }
        Ok(Scope {
            start: start.clone(),
            allowed,
            files_anywhere,
            #[cfg(feature = "test-support")]
            public: Vec::new(),
        })
    }

    /// **Tests only.** Treats `url`'s origin as a public site.
    #[cfg(feature = "test-support")]
    pub fn treat_as_public(mut self, url: &Url) -> Self {
        self.public.push(origin(url));
        self
    }

    /// Whether `url` is of the site: the start's origin, and its https
    /// twin when the start is plain http on the default port (review of
    /// #195: `http://host:8080` says nothing about `https://host`).
    pub fn in_site(&self, url: &Url) -> bool {
        if origin(url) == origin(&self.start) {
            return true;
        }
        self.start.scheme() == "http"
            && self.start.port().is_none()
            && url.scheme() == "https"
            && url.host_str() == self.start.host_str()
            && url.port().is_none()
    }

    fn allowed(&self, url: &Url) -> bool {
        self.allowed.iter().any(|a| a.matches(url))
    }

    fn public_for_tests(&self, _url: &Url) -> bool {
        #[cfg(feature = "test-support")]
        if self.public.contains(&origin(_url)) {
            return true;
        }
        false
    }

    /// Whether `url` may be requested, as a page (`as_page`) or as a file.
    /// `redirected_from` is the URL that redirected to it, if any.
    /// `any_origin_file`: a file that may be on another origin without
    /// `--files-anywhere`: the target of a page's redirect (owner's decision
    /// 3 on ADR-0010: downloads through a CDN work) and robots.txt.
    pub fn check(
        &self,
        url: &Url,
        as_page: bool,
        redirected_from: Option<&Url>,
        any_origin_file: bool,
    ) -> Result<Place, Refusal> {
        if !matches!(url.scheme(), "http" | "https") {
            return Err(Refusal::Scheme);
        }
        if !url.username().is_empty() || url.password().is_some() {
            return Err(Refusal::Credentials);
        }
        if redirected_from.is_some_and(|from| from.scheme() == "https") && url.scheme() != "https" {
            return Err(Refusal::Downgrade);
        }
        if is_trap(url) {
            return Err(Refusal::PathTrap);
        }
        let inside = self.in_site(url) || self.allowed(url);
        if !inside && is_local(url) && !self.public_for_tests(url) {
            return Err(Refusal::LocalAddress);
        }
        if inside {
            Ok(Place::Site)
        } else if !as_page && (self.files_anywhere || any_origin_file) {
            Ok(Place::FileElsewhere)
        } else {
            Err(Refusal::OtherSite)
        }
    }
}

/// Too many path segments, or one segment more than [`MAX_REPEATS`] times.
fn is_trap(url: &Url) -> bool {
    let Some(segments) = url.path_segments() else {
        return false;
    };
    let mut counts: HashMap<&str, usize> = HashMap::new();
    let mut total = 0;
    for segment in segments.filter(|s| !s.is_empty()) {
        total += 1;
        let count = counts.entry(segment).or_default();
        *count += 1;
        if total > MAX_SEGMENTS || *count > MAX_REPEATS {
            return true;
        }
    }
    false
}

#[cfg(test)]
mod tests {
    use super::*;

    fn url(text: &str) -> Url {
        Url::parse(text).unwrap()
    }

    fn scope(start: &str, allowed: &[&str], files_anywhere: bool) -> Scope {
        let allowed = allowed.iter().map(|a| a.parse().unwrap()).collect();
        Scope::new(&url(start), allowed, files_anywhere).unwrap()
    }

    #[test]
    fn the_site_is_the_start_origin() {
        let s = scope("https://Example.com/start/", &[], false);
        assert_eq!(
            s.check(&url("https://example.com/x.zip"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("https://example.com:443/x"), true, None, false),
            Ok(Place::Site)
        );
        for other in [
            "https://www.example.com/x",
            "https://example.com:8443/x",
            "http://example.com/x",
            "https://cdn.test/x.zip",
        ] {
            assert_eq!(
                s.check(&url(other), true, None, false),
                Err(Refusal::OtherSite),
                "{other}"
            );
            assert_eq!(
                s.check(&url(other), false, None, false),
                Err(Refusal::OtherSite),
                "{other}"
            );
        }
    }

    /// From http, the https twin of the same host is the site too; never
    /// the other way round.
    #[test]
    fn http_start_includes_its_https_twin() {
        let s = scope("http://example.com/", &[], false);
        assert_eq!(
            s.check(&url("https://example.com/a"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("https://example.com:8443/a"), true, None, false),
            Err(Refusal::OtherSite)
        );
        let s = scope("https://example.com/", &[], false);
        assert_eq!(
            s.check(&url("http://example.com/a"), true, None, false),
            Err(Refusal::OtherSite)
        );
        // A start on another port has no twin (review of #195).
        let s = scope("http://example.com:8080/", &[], false);
        for other in ["https://example.com/a", "https://example.com:8080/a"] {
            assert_eq!(
                s.check(&url(other), true, None, false),
                Err(Refusal::OtherSite),
                "{other}"
            );
        }
    }

    #[test]
    fn only_http_and_https_are_asked_for() {
        let s = scope("https://example.com/", &[], true);
        for other in [
            "ftp://example.com/x.zip",
            "file:///etc/passwd",
            "data:text/plain,x",
            "ws://example.com/x",
        ] {
            for as_page in [true, false] {
                assert_eq!(
                    s.check(&url(other), as_page, None, true),
                    Err(Refusal::Scheme),
                    "{other}"
                );
            }
        }
    }

    #[test]
    fn allowed_hosts_with_and_without_a_port() {
        let s = scope(
            "https://example.com/",
            &["www.example.com", "cdn.test:8080"],
            false,
        );
        assert_eq!(
            s.check(&url("https://www.example.com/p"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("http://www.example.com/p"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("https://www.example.com:444/p"), true, None, false),
            Err(Refusal::OtherSite)
        );
        assert_eq!(
            s.check(&url("http://cdn.test:8080/p"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("http://cdn.test/p"), true, None, false),
            Err(Refusal::OtherSite)
        );
    }

    #[test]
    fn allowed_host_parsing() {
        for (text, host, port) in [
            ("Example.COM", "example.com", None),
            ("example.com:80", "example.com", Some(80)),
            ("127.0.0.1:9000", "127.0.0.1", Some(9000)),
            ("[::1]:8080", "[::1]", Some(8080)),
            ("[::1]", "[::1]", None),
            ("2130706433", "127.0.0.1", None),
        ] {
            let a: AllowedHost = text.parse().unwrap();
            assert_eq!((a.host.as_str(), a.port), (host, port), "{text}");
        }
        for bad in ["", "a/b", "http://x", "u@h", "h:99999", "h:", "a b"] {
            assert!(bad.parse::<AllowedHost>().is_err(), "{bad}");
        }
    }

    #[test]
    fn files_anywhere_allows_files_only() {
        let s = scope("https://example.com/", &[], true);
        assert_eq!(
            s.check(&url("https://cdn.test/a.zip"), false, None, false),
            Ok(Place::FileElsewhere)
        );
        assert_eq!(
            s.check(&url("https://cdn.test/page"), true, None, false),
            Err(Refusal::OtherSite)
        );
    }

    /// Owner's decision on ADR-0010: a local address outside the site is
    /// refused, whatever its form, even with `--files-anywhere`.
    #[test]
    fn local_addresses_outside_the_site_are_refused() {
        let s = scope("https://example.com/", &[], true);
        for local in [
            "http://127.0.0.1/a.zip",
            "http://2130706433/a.zip",
            "http://0x7f.1/a.zip",
            "http://192.168.1.1/a.zip",
            "http://[::ffff:127.0.0.1]/a.zip",
            "http://localhost/a.zip",
            "http://169.254.169.254/latest",
        ] {
            assert_eq!(
                s.check(&url(local), false, None, false),
                Err(Refusal::LocalAddress),
                "{local}"
            );
        }
    }

    /// The exception is by origin: the start's own origin, or a host the
    /// user named; not another port or host of the same local network.
    #[test]
    fn the_local_exception_is_the_site_or_a_named_host() {
        let s = scope("http://192.168.1.10/", &["192.168.1.20:8080"], true);
        assert_eq!(
            s.check(&url("http://192.168.1.10/a.zip"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("https://192.168.1.10/a.zip"), true, None, false),
            Ok(Place::Site)
        );
        assert_eq!(
            s.check(&url("http://192.168.1.20:8080/a"), true, None, false),
            Ok(Place::Site)
        );
        for refused in [
            "http://192.168.1.10:6379/x.zip",
            "http://192.168.1.1/x.zip",
            "http://192.168.1.20/x.zip",
        ] {
            assert_eq!(
                s.check(&url(refused), false, None, false),
                Err(Refusal::LocalAddress),
                "{refused}"
            );
        }
    }

    #[test]
    fn a_redirect_from_https_to_http_is_refused() {
        let s = scope("http://example.com/", &[], true);
        let from = url("https://example.com/a");
        assert_eq!(
            s.check(&url("http://example.com/b"), true, Some(&from), false),
            Err(Refusal::Downgrade)
        );
        assert_eq!(
            s.check(&url("https://example.com/b"), true, Some(&from), false),
            Ok(Place::Site)
        );
    }

    #[test]
    fn path_traps_are_refused() {
        let s = scope("https://example.com/", &[], false);
        assert_eq!(
            s.check(
                &url("https://example.com/a/b/a/b/a/b/a/b"),
                true,
                None,
                false
            ),
            Err(Refusal::PathTrap)
        );
        assert_eq!(
            s.check(&url("https://example.com/a/b/a/b/a/b"), true, None, false),
            Ok(Place::Site)
        );
        let deep = format!(
            "https://example.com/{}",
            (0..33).map(|i| format!("d{i}/")).collect::<String>()
        );
        assert_eq!(
            s.check(&url(&deep), true, None, false),
            Err(Refusal::PathTrap)
        );
    }

    #[test]
    fn a_start_with_credentials_is_refused() {
        assert!(Scope::new(&url("https://u:p@example.com/"), vec![], false).is_err());
        assert!(Scope::new(&url("https://u@example.com/"), vec![], false).is_err());
    }
}
