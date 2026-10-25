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
/// host a segment is fetched from. The page (`Referer`) follows the
/// browsers' default rule, `strict-origin-when-cross-origin`: the whole
/// address to its own origin, only its origin (`https://site/`) to another
/// one, and nothing from https to http (review of #74).
#[derive(Clone, Default)]
pub struct FromBrowser {
    cookie: Option<(Origin, HeaderValue)>,
    referer: Option<Url>,
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
        let referer = referrer
            .and_then(|r| Url::parse(r).ok())
            .filter(|r| matches!(r.scheme(), "http" | "https") && r.host_str().is_some())
            .map(|mut r| {
                // Never part of a Referer (RFC 9110 §10.1.3).
                let _ = r.set_username("");
                let _ = r.set_password(None);
                r.set_fragment(None);
                r
            });
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
        if let Some(value) = self
            .referer
            .as_ref()
            .and_then(|page| referer_for(page, url))
        {
            request = request.header(header::REFERER, value);
        }
        request
    }
}

/// What `strict-origin-when-cross-origin` sends as the `Referer` of `page`
/// for a request to `to`.
fn referer_for(page: &Url, to: &Url) -> Option<HeaderValue> {
    if page.scheme() == "https" && to.scheme() != "https" {
        return None;
    }
    let text = if origin(page) == origin(to) {
        page.as_str().to_owned()
    } else {
        format!("{}/", page.origin().ascii_serialization())
    };
    HeaderValue::from_str(&text).ok()
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

    /// The browsers' default rule for the page: whole to its origin, the
    /// origin alone elsewhere, nothing from https to http.
    #[test]
    fn the_page_is_sent_as_browsers_send_it() {
        let from = FromBrowser::new(
            "https://cdn.test/f",
            None,
            Some("https://user:pw@example.com/private/page?session=ABC#top"),
        );
        let page = from.referer.as_ref().unwrap();
        let to = |u: &str| {
            referer_for(page, &Url::parse(u).unwrap()).map(|v| v.to_str().unwrap().to_owned())
        };
        assert_eq!(
            to("https://example.com/f.zip").as_deref(),
            Some("https://example.com/private/page?session=ABC")
        );
        assert_eq!(
            to("https://cdn.test/f.zip").as_deref(),
            Some("https://example.com/")
        );
        assert_eq!(to("http://example.com/f.zip"), None);
        let plain = FromBrowser::new("http://a.test/f", None, Some("http://a.test/p?x=1"));
        let page = plain.referer.as_ref().unwrap();
        assert_eq!(
            referer_for(page, &Url::parse("https://b.test/").unwrap()).unwrap(),
            "http://a.test/"
        );
        assert!(
            FromBrowser::new("https://a.test/f", None, Some("chrome://newtab/"))
                .referer
                .is_none()
        );
    }

    #[test]
    fn values_a_header_cannot_hold_are_left_out() {
        let from = FromBrowser::new(
            "https://example.com/a.zip",
            Some("a=1\r\nX: y"),
            Some("https://e.com/\n"),
        );
        assert!(!from.has_cookies());
        // The URL parser drops the line break (a browser does the same), so it
        // never reaches the header.
        assert_eq!(
            from.referer.as_ref().map(Url::as_str),
            Some("https://e.com/")
        );
        assert!(!FromBrowser::new("not a url", Some("a=1"), None).has_cookies());
    }
}
