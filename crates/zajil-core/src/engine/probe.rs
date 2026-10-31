//! What a link leads to before a download: the probe, and pages read as
//! text.

use std::io;

use reqwest::header;
use reqwest::{Response, StatusCode};

use crate::naming::{suggested_name, with_type_extension};

use super::download::Attempt;
use super::http::{ReadBody, parse_url, state_from_response, without_secrets};
use super::{Engine, Error, Stop};

/// A page [`Engine::fetch_page`] fetched (M7, the site grabber).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FetchedPage {
    /// The status the server answered, redirects included.
    pub status: u16,
    /// A redirect's `Location` (301, 302, 303, 307 or 308), as the server
    /// sent it: never followed, so the caller can check the target before
    /// asking for it. `None` for a redirect whose `Location` is missing or
    /// not text: a dead end.
    pub location: Option<String>,
    /// The media type, lowercase, without parameters (`text/html`).
    pub content_type: Option<String>,
    /// The name the file would get (as [`Probe::name`]).
    pub name: String,
    /// The body, read only for a successful HTML page (`text/html` or
    /// `application/xhtml+xml`; any successful answer for
    /// [`Engine::fetch_robots_until`]), and at most the size asked for.
    pub body: Option<Vec<u8>>,
    /// The body was longer than the size asked for: the rest was not read.
    pub truncated: bool,
}

/// What [`Engine::probe`] found.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Probe {
    /// The file's name, as a download would give it (sanitized).
    pub name: String,
    /// Its size, if the server said.
    pub size: Option<u64>,
    /// It can be split over several connections and resumed later.
    pub resumable: bool,
    /// The server announced `Accept-Ranges: bytes` (resumable also needs a
    /// size and a strong validator; the add dialog says which is missing).
    pub ranges: bool,
    /// The host of the final URL (after redirects), for categories.
    pub host: Option<String>,
    /// Its media type (`Content-Type` without parameters, lower case), if
    /// the server said: a sign-in page a link redirected to is `text/html`.
    pub content_type: Option<String>,
}

impl Probe {
    /// The server answered with a web page, not a file: a sign-in page a
    /// link redirected to, or a file's page on a site. Interfaces warn
    /// before downloading it.
    pub fn is_page(&self) -> bool {
        self.content_type.as_deref().is_some_and(is_page_type)
    }
}

/// Whether `media_type` is a web page: `text/html` or
/// `application/xhtml+xml`, in any case and with any parameters (a raw
/// `Content-Type` header, or [`Probe::content_type`]).
pub fn is_page_type(media_type: &str) -> bool {
    let media_type = media_type
        .split(';')
        .next()
        .unwrap_or_default()
        .trim()
        .to_ascii_lowercase();
    matches!(media_type.as_str(), "text/html" | "application/xhtml+xml")
}

/// The name a response's file gets: from its `Content-Disposition` or its
/// address, cleaned, with the extension of its `Content-Type` when it has
/// none (`view` sent as `text/html` is `view.html`).
pub(super) fn response_name(response: &Response) -> String {
    let name = suggested_name(
        response
            .headers()
            .get(header::CONTENT_DISPOSITION)
            .map(|v| v.as_bytes()),
        response.url(),
    );
    let content_type = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok());
    with_type_extension(name, content_type)
}

/// A web page sent as a page (`200`, `text/html` or
/// `application/xhtml+xml`, without `Content-Disposition`), not as a file:
/// a file, an `.html` one too, comes with `Content-Disposition` from Drive.
pub(super) fn is_bare_page(response: &Response) -> bool {
    let page = response
        .headers()
        .get(header::CONTENT_TYPE)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.split(';').next())
        .is_some_and(|t| is_page_type(&t.trim().to_ascii_lowercase()));
    response.status() == StatusCode::OK
        && page
        && !response.headers().contains_key(header::CONTENT_DISPOSITION)
}

/// The text of a page of at most `limit` bytes; `None` when it is larger
/// or not text.
pub(super) async fn read_page(
    mut response: Response,
    limit: usize,
) -> Result<Option<String>, Error> {
    let mut body = Vec::new();
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(|err| Error::Http(without_secrets(err)))?
    {
        if body.len().saturating_add(chunk.len()) > limit {
            return Ok(None);
        }
        body.extend_from_slice(&chunk);
    }
    Ok(String::from_utf8(body).ok())
}

impl Engine {
    /// A small text resource at `url` (an HLS playlist), at most `limit`
    /// bytes, with the same TLS, redirects and browser headers as a
    /// download, and not saved anywhere. For the video module's own DRM
    /// check (ADR-0009, section 6). A larger answer is an error, never
    /// read past the limit.
    pub async fn fetch_text(&self, url: &str, limit: usize, stop: &Stop) -> Result<String, Error> {
        let url = parse_url(url)?;
        let fetch = async {
            let mut response = self.get(url, None).await?;
            if response.status() != StatusCode::OK {
                return Err(Error::Status(response.status()));
            }
            let mut body = Vec::new();
            while let Some(chunk) = response
                .chunk()
                .await
                .map_err(|err| Error::Http(without_secrets(err)))?
            {
                if body.len().saturating_add(chunk.len()) > limit {
                    return Err(Error::Io(io::Error::other(format!(
                        "the answer is larger than {limit} bytes"
                    ))));
                }
                body.extend_from_slice(&chunk);
            }
            Ok(String::from_utf8_lossy(&body).into_owned())
        };
        tokio::select! {
            result = fetch => result,
            () = stop.requested() => Err(Error::Stopped { state_file: None }),
        }
    }

    /// What `url` holds, without downloading it: the name the file would
    /// get, its size, and whether it can be split and resumed (the server
    /// announces `Accept-Ranges: bytes`, a size and a strong validator).
    /// For the add dialog of the interface (M4). The first response is
    /// read for its headers only; the connection is then dropped.
    pub async fn probe(&self, url: &str) -> Result<Probe, Error> {
        self.probe_until(url, &Stop::new()).await
    }

    /// [`probe`](Self::probe), given up when `stop` is requested (the add
    /// dialog is closed while the server is tried again: review of #48).
    pub async fn probe_until(&self, url: &str, stop: &Stop) -> Result<Probe, Error> {
        let url = parse_url(url)?;
        let mut attempt = Attempt::new(Some(&url), stop, &self.limits());
        let response = self
            .retrying(&mut attempt, &mut |_| {}, || self.get(url.clone(), None))
            .await?;
        if response.status() != StatusCode::OK {
            return Err(Error::Status(response.status()));
        }
        let name = response_name(&response);
        let state = state_from_response(String::new(), false, &response);
        let size = response.content_length();
        let resumable = state.resumable();
        Ok(Probe {
            name,
            size,
            resumable,
            ranges: state.accept_ranges,
            host: response.url().host_str().map(str::to_owned),
            content_type: response
                .headers()
                .get(header::CONTENT_TYPE)
                .and_then(|v| v.to_str().ok())
                .and_then(|v| v.split(';').next())
                .map(|v| v.trim().to_ascii_lowercase())
                .filter(|v| !v.is_empty()),
        })
    }

    /// Fetches the page at `url` into memory, for the site grabber (M7,
    /// ADR-0010): **without following a redirect** (its `Location` is
    /// returned, for the caller to check), with the body read only for a
    /// successful HTML page, and at most `max_bytes` of it. One try; the
    /// same TLS checks, user agent and browser headers as a download.
    pub async fn fetch_page(&self, url: &str, max_bytes: usize) -> Result<FetchedPage, Error> {
        self.fetch_page_until(url, max_bytes, &Stop::new()).await
    }

    /// [`fetch_page`](Self::fetch_page), given up when `stop` is requested.
    pub async fn fetch_page_until(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
    ) -> Result<FetchedPage, Error> {
        self.fetch_page_inner(url, max_bytes, stop, ReadBody::Html)
            .await
    }

    /// Like [`fetch_page_until`](Self::fetch_page_until), but the body of
    /// any successful answer is read, whatever its type: for `robots.txt`,
    /// which servers send as `text/plain` or with no type at all. (Not
    /// [`fetch_text`](Self::fetch_text), which follows redirects and wants
    /// a 200.)
    pub async fn fetch_robots_until(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
    ) -> Result<FetchedPage, Error> {
        self.fetch_page_inner(url, max_bytes, stop, ReadBody::Any)
            .await
    }

    async fn fetch_page_inner(
        &self,
        url: &str,
        max_bytes: usize,
        stop: &Stop,
        read: ReadBody,
    ) -> Result<FetchedPage, Error> {
        let url = parse_url(url)?;
        let request = self.browser.apply(&url, self.page_client.get(url.clone()));
        let mut response = tokio::select! {
            result = request.send() => result.map_err(|err| Error::Http(without_secrets(err)))?,
            () = stop.requested() => return Err(Error::Stopped { state_file: None }),
        };
        let header_text = |name| {
            response
                .headers()
                .get(name)
                .and_then(|v: &header::HeaderValue| v.to_str().ok())
                .map(str::to_owned)
        };
        // Only a redirect's: a 300 or 304 that carries one leads nowhere.
        let location = header_text(header::LOCATION)
            .filter(|_| matches!(response.status().as_u16(), 301 | 302 | 303 | 307 | 308));
        let content_type = header_text(header::CONTENT_TYPE)
            .and_then(|v| v.split(';').next().map(|v| v.trim().to_ascii_lowercase()))
            .filter(|v| !v.is_empty());
        let name = response_name(&response);
        let html = matches!(
            content_type.as_deref(),
            Some("text/html" | "application/xhtml+xml")
        );
        let mut page = FetchedPage {
            status: response.status().as_u16(),
            location,
            content_type,
            name,
            body: None,
            truncated: false,
        };
        if !(response.status().is_success() && (html || read == ReadBody::Any)) {
            return Ok(page);
        }
        let mut body = Vec::new();
        loop {
            let chunk = tokio::select! {
                biased;
                () = stop.requested() => return Err(Error::Stopped { state_file: None }),
                chunk = response.chunk() => chunk.map_err(|err| Error::Http(without_secrets(err)))?,
            };
            let Some(chunk) = chunk else { break };
            let room = max_bytes.saturating_sub(body.len());
            if chunk.len() > room {
                body.extend_from_slice(&chunk[..room]);
                page.truncated = true;
                break;
            }
            body.extend_from_slice(&chunk);
        }
        page.body = Some(body);
        Ok(page)
    }
}
