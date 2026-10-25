//! What the browser extension adds to a download's requests (M5): the
//! browser's cookies for the download's address, and the page its link was
//! on. **Kept in memory only**, never in the state file (owner's decision
//! on #7): after a restart Zajil asks the extension again.

use reqwest::header::{self, HeaderValue};
use reqwest::{RequestBuilder, Url};

/// The browser's request headers for one download.
///
/// The cookies go only to the origin they were given for (the same scheme,
/// host and port): not to a CDN the server redirects to, nor to another
/// host a segment is fetched from. The `Referer` goes with every request,
/// as a browser sends it.
#[derive(Clone, Default)]
pub struct FromBrowser {
    cookie: Option<(Origin, HeaderValue)>,
    referer: Option<HeaderValue>,
}

/// A URL's scheme, host and port (the port the scheme implies, if none).
type Origin = (String, String, Option<u16>);

fn origin(url: &Url) -> Option<Origin> {
    Some((
        url.scheme().to_owned(),
        url.host_str()?.to_ascii_lowercase(),
        url.port_or_known_default(),
    ))
}

impl FromBrowser {
    /// The `cookies` (a `Cookie` header) the browser has for `url`, and the
    /// page `referrer`. Values a header cannot hold (control characters…)
    /// are left out, as is anything for a URL that does not parse.
    pub fn new(url: &str, cookies: Option<&str>, referrer: Option<&str>) -> Self {
        let origin = Url::parse(url).ok().as_ref().and_then(origin);
        let cookie = match (origin, cookies) {
            (Some(origin), Some(cookies)) if !cookies.is_empty() => {
                HeaderValue::from_str(cookies).ok().map(|mut value| {
                    value.set_sensitive(true);
                    (origin, value)
                })
            }
            _ => None,
        };
        let referer = referrer.and_then(|r| HeaderValue::from_str(r).ok());
        FromBrowser { cookie, referer }
    }

    /// Whether there are cookies to send.
    pub fn has_cookies(&self) -> bool {
        self.cookie.is_some()
    }

    /// `request`, to `url`, with these headers: the cookies only when `url`
    /// is of their origin.
    pub(crate) fn apply(&self, url: &Url, mut request: RequestBuilder) -> RequestBuilder {
        if let Some((theirs, cookie)) = &self.cookie
            && origin(url).as_ref() == Some(theirs)
        {
            request = request.header(header::COOKIE, cookie.clone());
        }
        if let Some(referer) = &self.referer {
            request = request.header(header::REFERER, referer.clone());
        }
        request
    }
}

/// Never shows the cookies.
impl std::fmt::Debug for FromBrowser {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("FromBrowser")
            .field("cookie", &self.cookie.as_ref().map(|_| "[hidden]"))
            .field("referer", &self.referer)
            .finish()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_cookies_are_never_shown() {
        let from = FromBrowser::new("https://example.com/a.zip", Some("sid=SECRET"), None);
        assert!(from.has_cookies());
        assert!(!format!("{from:?}").contains("SECRET"));
    }

    #[test]
    fn values_a_header_cannot_hold_are_left_out() {
        let from = FromBrowser::new(
            "https://example.com/a.zip",
            Some("a=1\r\nX: y"),
            Some("https://e.com/\n"),
        );
        assert!(!from.has_cookies());
        assert!(from.referer.is_none());
        assert!(!FromBrowser::new("not a url", Some("a=1"), None).has_cookies());
    }
}
