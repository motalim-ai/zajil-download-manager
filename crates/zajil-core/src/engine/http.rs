//! HTTP itself: the client, the requests and their tries, links shown
//! without their secrets, TLS, validators and `Content-Range`.

use std::io;
use std::time::SystemTime;

use reqwest::header::{self, HeaderMap};
use reqwest::{Client, Response, StatusCode, Url, redirect};

use crate::http_date;
use crate::state::{DownloadState, Validator};

use super::download::Attempt;
use super::probe::{is_bare_page, read_page};
use super::{Engine, Error, Event, Options};

/// The origin (`scheme://host[:port]`) the download's bytes came from: its
/// final address, else its link.
pub(super) fn origin_of(state: &DownloadState) -> Option<String> {
    [&state.final_url, &state.url]
        .into_iter()
        .filter_map(|u| Url::parse(u).ok())
        .map(|u| u.origin())
        .find(|o| o.is_tuple())
        .map(|o| o.ascii_serialization())
}

/// Which bodies [`Engine::fetch_page_inner`] reads.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum ReadBody {
    Html,
    Any,
}

/// Whether a client follows redirects ([`build_client`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Redirects {
    /// Up to [`Options::max_redirects`], never from https to http: every
    /// download, probe and refresh.
    Follow,
    /// Never: [`Engine::fetch_page`] returns the redirect itself.
    Never,
}

pub(super) fn build_client(
    options: &Options,
    extra_roots: &[Vec<u8>],
    redirects: Redirects,
) -> Result<Client, Error> {
    // rustls with the `ring` provider, and reqwest's built-in use of the
    // operating system's certificate store and checks
    // (`rustls-platform-verifier`). Nothing here can turn verification off.
    // Installing fails harmlessly if a provider is already installed.
    let _ = rustls::crypto::ring::default_provider().install_default();

    let max_redirects = options.max_redirects;
    let follow = redirect::Policy::custom(move |attempt| {
        let downgrade = attempt.url().scheme() != "https"
            && attempt.previous().iter().any(|url| url.scheme() == "https");
        if attempt.previous().len() > max_redirects {
            attempt.error(format!("more than {max_redirects} redirects"))
        } else if !matches!(attempt.url().scheme(), "http" | "https") {
            attempt.error("redirect to an unsupported scheme")
        } else if downgrade {
            attempt.error("refusing a redirect from https to http")
        } else {
            attempt.follow()
        }
    });

    let mut builder = Client::builder().tls_backend_rustls();
    for der in extra_roots {
        // Test builds only (see `Engine::with_test_root`).
        builder =
            builder.tls_certs_merge([reqwest::Certificate::from_der(der).map_err(Error::Http)?]);
    }
    let builder = builder
        .redirect(match redirects {
            Redirects::Follow => follow,
            Redirects::Never => redirect::Policy::none(),
        })
        // No `Referer` of our own: after a redirect reqwest would send the
        // redirecting link, with its `?sig=…` token, to the target (review
        // of #74). The browser's page is sent by `FromBrowser` instead.
        .referer(false)
        .connect_timeout(options.connect_timeout)
        .read_timeout(options.idle_timeout)
        // Never one picked up implicitly from the system or the
        // environment: only the user's own.
        .no_proxy();
    let builder = match &options.proxy {
        Some(proxy) => {
            crate::settings::check_proxy(proxy).map_err(|e| Error::Io(std::io::Error::other(e)))?;
            builder.proxy(reqwest::Proxy::all(proxy.as_str()).map_err(Error::Http)?)
        }
        None => builder,
    };
    builder
        .user_agent(concat!("Zajil/", env!("CARGO_PKG_VERSION")))
        .build()
        .map_err(Error::Http)
}

pub(crate) fn parse_url(raw: &str) -> Result<Url, Error> {
    let url = Url::parse(raw).map_err(|err| Error::InvalidUrl {
        url: shown_url(raw),
        reason: err.to_string(),
    })?;
    match url.scheme() {
        "http" | "https" => Ok(url),
        other => Err(Error::UnsupportedScheme(other.to_owned())),
    }
}

/// `raw` (a URL that did not parse) as an error message may show it:
/// without what may be a user name and password ([`without_userinfo`]),
/// and without the query and fragment, which may hold a token (`?sig=…`).
/// Tokens are never printed (owner's decision on #28).
pub(super) fn shown_url(raw: &str) -> String {
    let mut shown = without_userinfo(raw);
    if let Some(end) = shown.find(['?', '#']) {
        shown.truncate(end);
    }
    shown
}

/// `err` with the secrets its URL may hold removed, so its message never
/// shows them: the user name and password, the query (`?sig=…`) and the
/// fragment.
pub(crate) fn without_secrets(mut err: reqwest::Error) -> reqwest::Error {
    if let Some(url) = err.url_mut() {
        let _ = url.set_username("");
        let _ = url.set_password(None);
        url.set_query(None);
        url.set_fragment(None);
    }
    err
}

/// The message of an HTTP error, without the secrets of its URL.
pub(crate) fn http_error(err: reqwest::Error) -> String {
    error_chain(&without_secrets(err))
}

/// Whether an HTTP error may pass if the request is sent again: a
/// connection that could not be made or broke, or a timeout. A certificate
/// that cannot be trusted does not pass by itself and is never retried;
/// nor is a refused redirect.
fn may_pass(err: &reqwest::Error) -> bool {
    if err.is_builder() || err.is_redirect() || is_tls_error(err) {
        return false;
    }
    err.is_timeout() || err.is_connect() || err.is_request() || err.is_body()
}

/// Whether a TLS error (such as an untrusted certificate) is behind `err`.
/// `io::Error` skips its own inner error in `source()`, and the TLS error
/// sits in an `io::Error` inside another one, so go into each `io::Error`
/// instead of asking for its source.
fn is_tls_error(err: &(dyn std::error::Error + 'static)) -> bool {
    let mut next = Some(err);
    while let Some(err) = next {
        if err.is::<rustls::Error>() {
            return true;
        }
        next = match err.downcast_ref::<io::Error>() {
            Some(io) => io
                .get_ref()
                .map(|inner| inner as &(dyn std::error::Error + 'static)),
            None => err.source(),
        };
    }
    false
}

/// `raw` (a URL that did not parse) with what may be a user name and
/// password hidden, so an error message never shows them: everything up to
/// the last `@`, except a leading `http:` or `https:` and its slashes. It
/// does not rely on `://`, which a mistyped URL may lack (`http:/u:p@h`) or
/// have again later (`?next=https://…`); and anything before the `@` that
/// is not that scheme may be a token (`TOKEN:x-oauth-basic@h`, `http//…`).
/// A password may hold `/`, `#` or `?`, so the last `@` is the only safe
/// end; an `@` later in the URL hides a little too much, never too little.
fn without_userinfo(raw: &str) -> String {
    let Some(at) = raw.rfind('@') else {
        return raw.to_owned();
    };
    let head = &raw[..at];
    let scheme = ["https:", "http:"].into_iter().find(|scheme| {
        head.get(..scheme.len())
            .is_some_and(|start| start.eq_ignore_ascii_case(scheme))
    });
    let keep = match scheme {
        Some(scheme) => {
            let rest = &head[scheme.len()..];
            scheme.len() + rest.len() - rest.trim_start_matches('/').len()
        }
        None => 0,
    };
    format!("{}***{}", &raw[..keep], &raw[at..])
}

pub(super) fn header_str(headers: &HeaderMap, name: header::HeaderName) -> Option<&str> {
    headers.get(name).and_then(|v| v.to_str().ok())
}

fn validator_from(headers: &HeaderMap) -> Validator {
    Validator {
        etag: header_str(headers, header::ETAG).map(str::to_owned),
        last_modified: header_str(headers, header::LAST_MODIFIED).map(str::to_owned),
        date: header_str(headers, header::DATE).map(str::to_owned),
    }
}

/// The first state for a whole-file response. `had_credentials`: the URL
/// the user gave had a user name or password (never saved themselves).
pub(super) fn state_from_response(
    url: String,
    had_credentials: bool,
    response: &Response,
) -> DownloadState {
    let headers = response.headers();
    let accept_ranges = header_str(headers, header::ACCEPT_RANGES)
        .is_some_and(|v| v.split(',').any(|u| u.trim().eq_ignore_ascii_case("bytes")));
    let mut state = DownloadState::new(
        url,
        without_credentials(response.url()),
        validator_from(headers),
        response.content_length(),
        accept_ranges,
    );
    state.had_credentials = had_credentials;
    state
}

/// The `If-Range` value RFC 9110 §13.1.5 allows, or `None` when there is no
/// strong validator and the download must start over rather than risk
/// mixing two versions of the file:
/// - a strong `ETag` (a weak one, `W/"…"`, must never be sent);
/// - only when there is **no** `ETag` at all, `Last-Modified`, and only if
///   it is strong (§8.8.2.2): at least one second before the `Date` of the
///   same response. A file changed twice within that second would otherwise
///   look unchanged.
pub(crate) fn if_range_value(validator: &Validator) -> Option<String> {
    match &validator.etag {
        Some(etag) if !etag.trim_start().starts_with("W/") => Some(etag.clone()),
        Some(_weak) => None,
        None => {
            let last_modified = validator.last_modified.as_ref()?;
            let modified = http_date::parse(last_modified)?;
            let date = http_date::parse(validator.date.as_ref()?)?;
            let gap = date.checked_sub(modified)?;
            (gap >= 1).then(|| last_modified.clone())
        }
    }
}

/// A `206` continues our download only if its `Content-Range` starts where
/// our data ends, covers the same total size, and the validator matches.
pub(super) fn continues_at(response: &Response, state: &DownloadState, done: u64) -> bool {
    let range = header_str(response.headers(), header::CONTENT_RANGE).and_then(parse_content_range);
    let Some(ContentRange { start, end, total }) = range else {
        return false;
    };
    let same_total = match (total, state.content_length) {
        (Some(total), Some(expected)) => total == expected && end + 1 == total,
        (None, None) => true,
        _ => false,
    };
    start == done && same_total && !changed(state, response)
}

/// Whether `response` describes another version of the file than `state`.
pub(super) fn changed(state: &DownloadState, response: &Response) -> bool {
    changed_from(&state.validator, state.content_length, response)
}

/// Whether `response` describes another version of the file than the one
/// with `validator` and `content_length`.
pub(crate) fn changed_from(
    validator: &Validator,
    content_length: Option<u64>,
    response: &Response,
) -> bool {
    let now = validator_from(response.headers());
    let differs = |old: &Option<String>, new: &Option<String>| matches!((old, new), (Some(old), Some(new)) if old != new);
    differs(&validator.etag, &now.etag)
        || differs(&validator.last_modified, &now.last_modified)
        || (response.status() == StatusCode::OK
            && response.content_length().is_some()
            && response.content_length() != content_length)
}

/// A parsed `Content-Range: bytes start-end/total` (RFC 9110 §14.4).
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct ContentRange {
    pub start: u64,
    pub end: u64,
    pub total: Option<u64>,
}

pub(crate) fn parse_content_range(value: &str) -> Option<ContentRange> {
    let rest = value.trim().strip_prefix("bytes ")?;
    let (range, total) = rest.split_once('/')?;
    let (start, end) = range.split_once('-')?;
    let (start, end): (u64, u64) = (start.trim().parse().ok()?, end.trim().parse().ok()?);
    let total = match total.trim() {
        "*" => None,
        n => Some(n.parse().ok()?),
    };
    (start <= end && total.is_none_or(|t| end < t)).then_some(ContentRange { start, end, total })
}

/// `url` without a user name or password. Credentials in a URL are used for
/// the request but never written to the state file (ADR-0004, rule 7).
pub(super) fn without_credentials(url: &Url) -> String {
    let mut url = url.clone();
    let _ = url.set_username("");
    let _ = url.set_password(None);
    url.to_string()
}

/// An error and all its sources, so TLS and connection details reach the user.
pub(crate) fn error_chain(err: &dyn std::error::Error) -> String {
    let mut text = err.to_string();
    let mut source = err.source();
    while let Some(cause) = source {
        let cause_text = cause.to_string();
        if !text.contains(&cause_text) {
            text.push_str(": ");
            text.push_str(&cause_text);
        }
        source = cause.source();
    }
    text
}

impl Engine {
    /// Requests `final_url`, and the original URL if that fails: signed CDN
    /// links often expire, while the original URL redirects to a fresh one.
    /// With `attempt`'s credentials, if it has any for that origin: the
    /// state file never holds them, but a restart within one call does.
    /// Failures that may pass are tried again ([`Engine::retrying`]).
    pub(super) async fn get_with_fallback(
        &self,
        state: &DownloadState,
        range: Option<(u64, Option<u64>, String)>,
        attempt: &mut Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
    ) -> Result<Response, Error> {
        let final_url = attempt.authorize(&parse_url(&state.final_url)?);
        let url = attempt.authorize(&parse_url(&state.url)?);
        let same = state.final_url == state.url;
        self.retrying(attempt, on_event, || {
            self.get_either(&final_url, &url, same, range.clone())
        })
        .await
    }

    /// One try of [`Engine::get_with_fallback`].
    async fn get_either(
        &self,
        final_url: &Url,
        url: &Url,
        same: bool,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let first = self.get(final_url.clone(), range.clone()).await;
        let usable = |r: &Result<Response, Error>| {
            r.as_ref().is_ok_and(|r| {
                matches!(
                    r.status(),
                    StatusCode::OK
                        | StatusCode::PARTIAL_CONTENT
                        | StatusCode::RANGE_NOT_SATISFIABLE
                )
            })
        };
        if usable(&first) || same {
            return first;
        }
        self.get(url.clone(), range).await
    }

    /// Sends a request with `send`, and again after a growing wait while it
    /// fails for a reason that may pass: a connection that could not be
    /// made or broke, a timeout, `5xx`, `408 Request Timeout` or `429 Too
    /// Many Requests`. A `429` waits for its `Retry-After` (seconds or an
    /// HTTP date); one that asks for more than
    /// [`crate::retry::LONGEST_RETRY_AFTER`] is not waited for
    /// ([`Error::ServerBusy`]). After `max_failures` failures in a row the
    /// last answer is returned as it is. A certificate that cannot be
    /// trusted, a redirect that is refused or another status (such as
    /// `403` or `404`) is never retried. Same policy as the connections of
    /// a segmented download (M2).
    pub(super) async fn retrying<F: Future<Output = Result<Response, Error>>>(
        &self,
        attempt: &mut Attempt,
        on_event: &mut (dyn FnMut(Event) + Send),
        mut send: impl FnMut() -> F,
    ) -> Result<Response, Error> {
        loop {
            let result = tokio::select! {
                result = send() => result,
                () = attempt.stop.requested() => return Err(Error::Stopped { state_file: None }),
            };
            let (reason, asked) = match &result {
                Ok(response) if response.status() == StatusCode::TOO_MANY_REQUESTS => {
                    let asked = header_str(response.headers(), header::RETRY_AFTER)
                        .and_then(|v| crate::retry::retry_after(v, SystemTime::now()));
                    if let Some(wait) =
                        asked.filter(|&wait| wait > crate::retry::LONGEST_RETRY_AFTER)
                    {
                        return Err(Error::ServerBusy { wait });
                    }
                    (format!("the server answered {}", response.status()), asked)
                }
                Ok(response)
                    if response.status().is_server_error()
                        || response.status() == StatusCode::REQUEST_TIMEOUT =>
                {
                    (format!("the server answered {}", response.status()), None)
                }
                Ok(_) => return result,
                Err(Error::Http(err)) if may_pass(err) => (error_chain(err), None),
                Err(_) => return result,
            };
            if !attempt.failed(self.options.max_failures) {
                return result;
            }
            let wait = asked.unwrap_or_else(|| crate::retry::backoff(attempt.failures));
            if let Ok(response) = &result
                && response.status() == StatusCode::TOO_MANY_REQUESTS
            {
                on_event(Event::ServerBusy {
                    status: 429,
                    connections: 1,
                    wait: Some(wait),
                });
            }
            drop(result);
            on_event(Event::Retrying { wait, reason });
            tokio::select! {
                () = tokio::time::sleep(wait) => {}
                () = attempt.stop.requested() => return Err(Error::Stopped { state_file: None }),
            }
        }
    }

    /// GETs `url`; with `range`, only bytes `from` to `to` (or to the end)
    /// and only if the file still matches `If-Range`. A Google Drive
    /// file's page is asked for at its download address, and the page
    /// Drive answers a large file with is followed to its link, once (see
    /// [`crate::drive`]). Any other page that comes for Drive's address
    /// (a sign-in page it redirects a private file to, a page saying the
    /// file was downloaded too often) is [`Error::DrivePage`]: never the
    /// file.
    pub(super) async fn get(
        &self,
        url: Url,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let url = self.drive.direct_link(&url).unwrap_or(url);
        let response = self.send(url.clone(), range.clone()).await?;
        // Only what is asked of Drive's download address is Drive's: a page
        // from anywhere else is the page the user asked for.
        if !(self.drive.is_download(&url) && is_bare_page(&response)) {
            return Ok(response);
        }
        // Drive's own page, at its own address, may lead to the file;
        // one it redirected to elsewhere (a sign-in page) never does.
        if !self.drive.is_download(response.url()) {
            return Err(Error::DrivePage);
        }
        let page_url = response.url().clone();
        let page = read_page(response, crate::drive::MAX_PAGE_BYTES).await?;
        let link = page
            .and_then(|page| self.drive.confirmed_link(&page, &page_url, &url))
            .ok_or(Error::DrivePage)?;
        let response = self.send(link, range).await?;
        if is_bare_page(&response) {
            return Err(Error::DrivePage);
        }
        Ok(response)
    }

    /// Whether `state` is a Google Drive download: its link is a file's
    /// page or its download address, or its file came from there.
    pub(super) fn is_drive_download(&self, state: &DownloadState) -> bool {
        [&state.url, &state.final_url].iter().any(|u| {
            Url::parse(u)
                .is_ok_and(|u| self.drive.direct_link(&u).is_some() || self.drive.is_download(&u))
        })
    }

    /// One GET of `url`, as it is.
    async fn send(
        &self,
        url: Url,
        range: Option<(u64, Option<u64>, String)>,
    ) -> Result<Response, Error> {
        let mut request = self.browser.apply(&url, self.client.get(url.clone()));
        if let Some((from, to, if_range)) = range {
            let to = to.map(|to| to.to_string()).unwrap_or_default();
            request = request
                .header(header::RANGE, format!("bytes={from}-{to}"))
                .header(header::IF_RANGE, if_range);
        }
        request
            .send()
            .await
            .map_err(|err| Error::Http(without_secrets(err)))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const FIVE_GIB: u64 = 5 * 1024 * 1024 * 1024;

    #[test]
    fn parses_content_range() {
        assert_eq!(
            parse_content_range("bytes 100-999/1000"),
            Some(ContentRange {
                start: 100,
                end: 999,
                total: Some(1000)
            })
        );
        assert_eq!(
            parse_content_range("bytes 0-9/*"),
            Some(ContentRange {
                start: 0,
                end: 9,
                total: None
            })
        );
        for bad in [
            "bytes 9-0/10",
            "bytes 0-10/10",
            "bytes */10",
            "items 0-1/2",
            "bytes 0-x/2",
            "",
        ] {
            assert_eq!(parse_content_range(bad), None, "{bad:?}");
        }
    }

    #[test]
    fn handles_offsets_beyond_4_gib() {
        // Files larger than 4 GiB need 64-bit offsets end to end.
        let start = 4_500_000_000;
        let header = format!("bytes {start}-{}/{FIVE_GIB}", FIVE_GIB - 1);
        assert_eq!(
            parse_content_range(&header),
            Some(ContentRange {
                start,
                end: FIVE_GIB - 1,
                total: Some(FIVE_GIB)
            })
        );

        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("huge.iso.zajil-state");
        let mut state = DownloadState::new(
            "http://h/huge.iso".into(),
            "http://h/huge.iso".into(),
            Validator {
                etag: Some("\"e\"".into()),
                last_modified: None,
                date: None,
            },
            Some(FIVE_GIB),
            true,
        );
        state.segments[0].done = start;
        state.save(&path).unwrap();
        let loaded = DownloadState::load(&path).unwrap();
        assert_eq!(loaded.bytes_done(), start);
        assert_eq!(loaded.segments[0].end, Some(FIVE_GIB - 1));
    }

    fn validator(etag: Option<&str>, modified: Option<&str>, date: Option<&str>) -> Validator {
        Validator {
            etag: etag.map(str::to_owned),
            last_modified: modified.map(str::to_owned),
            date: date.map(str::to_owned),
        }
    }

    const MODIFIED: &str = "Wed, 21 Oct 2026 07:28:00 GMT";
    const ONE_SECOND_LATER: &str = "Wed, 21 Oct 2026 07:28:01 GMT";

    #[test]
    fn if_range_uses_a_strong_etag() {
        let v = validator(Some("\"a\""), Some(MODIFIED), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&v).as_deref(), Some("\"a\""));
    }

    #[test]
    fn if_range_never_uses_a_weak_etag_nor_falls_back_to_the_date() {
        // Review of #17: the date was sent next to a weak ETag.
        for weak in ["W/\"a\"", " W/\"a\""] {
            let v = validator(Some(weak), Some(MODIFIED), Some(ONE_SECOND_LATER));
            assert_eq!(if_range_value(&v), None, "{weak}");
        }
    }

    #[test]
    fn if_range_uses_the_date_only_when_it_is_strong() {
        let strong = validator(None, Some(MODIFIED), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&strong).as_deref(), Some(MODIFIED));
        // Same second as the response: weak (RFC 9110 §8.8.2.2).
        let same_second = validator(None, Some(MODIFIED), Some(MODIFIED));
        assert_eq!(if_range_value(&same_second), None);
        // Unknown or unreadable Date: cannot prove it is strong.
        assert_eq!(if_range_value(&validator(None, Some(MODIFIED), None)), None);
        let garbage = validator(None, Some(MODIFIED), Some("soon"));
        assert_eq!(if_range_value(&garbage), None);
        let unreadable = validator(None, Some("last week"), Some(ONE_SECOND_LATER));
        assert_eq!(if_range_value(&unreadable), None);
        // Date before Last-Modified (clock trouble): weak.
        let backwards = validator(None, Some(ONE_SECOND_LATER), Some(MODIFIED));
        assert_eq!(if_range_value(&backwards), None);
        // Absurd dates are ignored, never compared (no overflow).
        let far = "Sun, 06 Nov 9223372036854775807 08:49:37 GMT";
        assert_eq!(
            if_range_value(&validator(None, Some(MODIFIED), Some(far))),
            None
        );
        assert_eq!(
            if_range_value(&validator(None, Some(far), Some(MODIFIED))),
            None
        );
        assert_eq!(if_range_value(&Validator::default()), None);
    }

    #[test]
    fn an_invalid_url_never_shows_its_password() {
        // Review of #29: the whole URL, password included, was in the error.
        for raw in [
            "http://u:S3cret@127.0.0.1:99999/f.bin",
            "http://u:pa#S3cret@127.0.0.1:1/f.bin",
            "http://u:pa?S3cret@h:1/f",
            "http://u:pa/S3cret@h:99999/f",
            "u:S3cret@h:99999/f",
        ] {
            let message = parse_url(raw).unwrap_err().to_string();
            assert!(!message.contains("S3cret"), "{message}");
        }
        assert_eq!(
            without_userinfo("http://u:S3cret@h:99999/f"),
            "http://***@h:99999/f"
        );
        // Review of #29: a `://` only later in the URL, or none at all.
        for raw in [
            "http:/u:S3cret@h:99999/f?next=https://x",
            "http:/u:S3cret@h:99999/f#https://x",
            "http:u:S3cret@h:99999/f",
            // Third review of #29: a token as the user name, and no scheme
            // or a mistyped one.
            "S3cret_TOKEN:x-oauth-basic@h:99999/f",
            "http//S3cret_TOKEN:x-oauth-basic@h:99999/f",
            "https;//S3cret_TOKEN:x@h:99999/f",
            "HTTPS://S3cret:x@h:99999/f",
        ] {
            assert!(!without_userinfo(raw).contains("S3cret"), "{raw}");
            if let Err(err) = parse_url(raw) {
                assert!(!err.to_string().contains("S3cret"), "{err}");
            }
        }
        assert_eq!(without_userinfo("http://h:99999/f"), "http://h:99999/f");
        assert_eq!(without_userinfo("http:/u:p@h:1/f"), "http:/***@h:1/f");
        assert_eq!(without_userinfo("T:x@h:1/f"), "***@h:1/f");
    }

    #[test]
    fn an_invalid_url_never_shows_a_token() {
        // Review of #29 (owner's decision on #28): `?sig=…` was shown.
        for raw in [
            "http://h:99999/f.bin?sig=S3cret",
            "http://h:99999/f.bin#S3cret",
            "http://u:p@h:99999/f.bin?a=1&sig=S3cret#x",
        ] {
            let message = parse_url(raw).unwrap_err().to_string();
            assert!(!message.contains("S3cret"), "{message}");
            assert!(message.contains("h:99999/f.bin"), "{message}");
        }
        // An `@` in the query hides more than needed, never less.
        let message = parse_url("http://h:99999/f.bin?next=http://u:p@x/?S3cret")
            .unwrap_err()
            .to_string();
        assert!(!message.contains("S3cret"), "{message}");
        assert_eq!(shown_url("http://h:1/f?sig=x#y"), "http://h:1/f");
        assert_eq!(shown_url("http://h:1/f"), "http://h:1/f");
    }
}
