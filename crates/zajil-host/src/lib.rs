//! The Native Messaging protocol between Zajil's browser extension and
//! Zajil (ADR-0005), shared by `zajil-host` and the desktop app.
//!
//! The browser starts `zajil-host` and talks to it through stdin and
//! stdout: each message is JSON, preceded by its length in 4 bytes in the
//! computer's own byte order (Chrome and Firefox documentation). The host
//! checks every message against a fixed schema ([`Request`]), then hands
//! it to the running app over the user's private channel
//! (`zajil_platform::ipc`), in the same framing, and passes the app's
//! answer ([`Response`]) back.
//!
//! What is refused, before anything reaches the app:
//! - a message longer than [`MAX_REQUEST`] (read no further: the stream
//!   cannot be trusted after it);
//! - text that is not JSON, a field the schema does not know, a missing
//!   field, a `type` it does not know;
//! - another [`PROTOCOL_VERSION`];
//! - a URL that adding it to a queue would refuse (http or https only, no
//!   user name or password, nothing that drives a terminal or reorders
//!   text: `zajil_core::queue::check_url`), or longer than [`MAX_URL`];
//! - a referrer that is not such a URL; a file name with a path, control
//!   characters, or more than [`MAX_NAME`] bytes; cookies longer than
//!   [`MAX_COOKIES`] or with anything but visible ASCII and spaces (a line
//!   break would add a header of its own to the request).

use std::collections::HashMap;
use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use serde::{Deserialize, Serialize};

/// The protocol's version: an extension and an app of different versions
/// say so clearly instead of misreading each other.
pub const PROTOCOL_VERSION: u32 = 1;
/// How long one side waits for the other on the private channel.
pub const CHANNEL_TIMEOUT: Duration = Duration::from_secs(10);
/// How many connections the app serves at once; more wait their turn by
/// being refused (the host says so).
pub const MAX_CONNECTIONS: usize = 8;

/// The largest message the host reads from the browser.
pub const MAX_REQUEST: u32 = 64 * 1024;
/// The largest message the host may send to the browser (Chrome and
/// Firefox limit it to 1 MiB).
pub const MAX_RESPONSE: u32 = 1024 * 1024;
/// The longest URL or referrer, in bytes.
pub const MAX_URL: usize = 8 * 1024;
/// The longest suggested file name, in bytes.
pub const MAX_NAME: usize = 255;
/// The longest `Cookie` header value, in bytes.
pub const MAX_COOKIES: usize = 16 * 1024;
/// The longest extension instance id and cookie store id, in bytes.
pub const MAX_ID: usize = 64;
/// The most tickets a `keep` message lists.
pub const MAX_TICKETS: usize = 10_000;
/// How long the host waits for the extension to answer a question from
/// the app on the link; it always answers the app within it.
pub const LINK_ANSWER: Duration = Duration::from_secs(10);

/// A message from the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// Is the host there, and of which version?
    Ping { protocol_version: u32 },
    /// Show the app's window: sent by Zajil started a second time on the
    /// same data folder, on macOS (#60: instead of a socket at a fixed
    /// path in `/tmp`), never by the extension (`zajil-host` refuses it).
    ShowWindow { protocol_version: u32 },
    /// Download this.
    Download {
        protocol_version: u32,
        url: String,
        /// The page the link was on.
        #[serde(default)]
        referrer: Option<String>,
        /// The name the browser would have given the file.
        #[serde(default)]
        filename: Option<String>,
        /// The `Cookie` header for this URL only (ADR-0005).
        #[serde(default)]
        cookies: Option<String>,
        /// The extension's own random id (one per browser profile), so
        /// Zajil can ask that browser for the cookies again after a
        /// restart (M5).
        #[serde(default)]
        instance: Option<String>,
        /// The cookie store the cookies came from (a Firefox container).
        #[serde(default)]
        store: Option<String>,
        /// This download's random ticket: the extension gives its cookies
        /// again only for an open download's ticket (owner's decision on
        /// #80).
        #[serde(default)]
        ticket: Option<String>,
        /// From a private window: Zajil keeps nothing of it once it is
        /// done (security review #84).
        #[serde(default)]
        private: bool,
    },
    /// Download the video on this page (M6, ADR-0009): the user chose
    /// "Download video with Zajil" on a video, or on the page. Zajil asks
    /// yt-dlp for its formats, and the user chooses.
    Video {
        protocol_version: u32,
        /// The page the video is on: yt-dlp reads it.
        page: String,
        /// The video's own address, when it is an http(s) one (not a
        /// `blob:` the page made): for a page yt-dlp does not know.
        #[serde(default)]
        media: Option<String>,
        /// The `Cookie` header for the page only (ADR-0005), when the user
        /// allowed cookies; never from a private window.
        #[serde(default)]
        cookies: Option<String>,
        /// From a private window: Zajil keeps nothing of it once it is
        /// done, as for [`Request::Download`].
        #[serde(default)]
        private: bool,
    },
    /// The last private window closed (review of #91): Zajil takes every
    /// private window's download out of its queues and deletes their part
    /// and state files.
    PrivateClosed { protocol_version: u32 },
    /// Keep this connection open (`runtime.connectNative`): the link on
    /// which Zajil asks this browser for cookies again (M5). Only the
    /// host's own connection to the app carries it.
    Link {
        protocol_version: u32,
        instance: String,
    },
    /// The extension's answer on the link to a [`ToExtension`] question:
    /// for [`ToExtension::CookiesWanted`], the `Cookie` header, or `null`
    /// (no permission, no open download with that ticket, or none); for
    /// [`ToExtension::Keep`], always `null`.
    Cookies {
        protocol_version: u32,
        ask: u64,
        #[serde(default)]
        cookies: Option<String>,
    },
}

/// What the app asks the extension on the link (through the host).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum ToExtension {
    /// The cookies for `url` from cookie store `store`, for a download
    /// that came from this browser with its cookies before a restart.
    CookiesWanted {
        protocol_version: u32,
        ask: u64,
        url: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        store: Option<String>,
        /// The download's ticket, as the extension gave it.
        ticket: String,
    },
    /// Only these downloads' tickets are still open (not done, not
    /// removed): the extension forgets every other one at once.
    Keep {
        protocol_version: u32,
        ask: u64,
        tickets: Vec<String>,
    },
}

/// What the host tells the app on the link: the extension's answer, or
/// that none came within [`LINK_ANSWER`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum FromLink {
    Cookies {
        protocol_version: u32,
        ask: u64,
        #[serde(default)]
        cookies: Option<String>,
    },
    NoAnswer {
        protocol_version: u32,
        ask: u64,
    },
}

impl ToExtension {
    /// Reads and checks a question from the app.
    pub fn parse(message: &[u8]) -> Result<Self, String> {
        let question: ToExtension =
            serde_json::from_slice(message).map_err(|e| format!("not a Zajil question: {e}"))?;
        match &question {
            ToExtension::CookiesWanted {
                protocol_version,
                url,
                store,
                ticket,
                ..
            } => {
                check_version(*protocol_version)?;
                check_url(url, "URL")?;
                if let Some(store) = store {
                    check_id(store, "cookie store")?;
                }
                check_id(ticket, "ticket")?;
            }
            ToExtension::Keep {
                protocol_version,
                tickets,
                ..
            } => {
                check_version(*protocol_version)?;
                if tickets.len() > MAX_TICKETS {
                    return Err(format!("more than {MAX_TICKETS} tickets"));
                }
                for ticket in tickets {
                    check_id(ticket, "ticket")?;
                }
            }
        }
        Ok(question)
    }

    /// The question's number, which its answer carries.
    pub fn ask(&self) -> u64 {
        match self {
            ToExtension::CookiesWanted { ask, .. } | ToExtension::Keep { ask, .. } => *ask,
        }
    }
}

impl FromLink {
    /// Reads and checks the host's answer.
    pub fn parse(message: &[u8]) -> Result<Self, String> {
        let answer: FromLink =
            serde_json::from_slice(message).map_err(|e| format!("not a Zajil answer: {e}"))?;
        let version = match &answer {
            FromLink::Cookies {
                protocol_version,
                cookies,
                ..
            } => {
                if let Some(cookies) = cookies {
                    check_cookies(cookies)?;
                }
                *protocol_version
            }
            FromLink::NoAnswer {
                protocol_version, ..
            } => *protocol_version,
        };
        check_version(version)?;
        Ok(answer)
    }
}

/// The answer to a [`Request`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Response {
    pub protocol_version: u32,
    pub ok: bool,
    /// Why it was refused or failed.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// Zajil's language, `ar` or `en`, in the answer to a ping only: the
    /// extension's video button speaks it (owner's decision, 2026-10-27).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub language: Option<String>,
}

impl Response {
    pub fn ok() -> Self {
        Response {
            protocol_version: PROTOCOL_VERSION,
            ok: true,
            error: None,
            language: None,
        }
    }

    pub fn refused(why: impl Into<String>) -> Self {
        Response {
            protocol_version: PROTOCOL_VERSION,
            ok: false,
            error: Some(why.into()),
            language: None,
        }
    }

    /// The answer to a ping, with Zajil's language when its settings could
    /// be read.
    pub fn pong(language: Option<&str>) -> Self {
        Response {
            language: language.map(str::to_owned),
            ..Response::ok()
        }
    }
}

/// Why a message could not be read.
#[derive(Debug, thiserror::Error)]
pub enum FrameError {
    #[error("the message is {0} bytes; at most {1} are read")]
    TooLarge(u32, u32),
    #[error("the message ended early: {0}")]
    Io(#[from] io::Error),
}

/// Reads one message: `None` when the stream ends between messages. A
/// length above `max` is refused before reading the message itself.
pub fn read_frame(input: &mut impl Read, max: u32) -> Result<Option<Vec<u8>>, FrameError> {
    let mut length = [0u8; 4];
    let mut got = 0;
    while got < 4 {
        match input.read(&mut length[got..]) {
            Ok(0) if got == 0 => return Ok(None),
            Ok(0) => return Err(io::Error::from(io::ErrorKind::UnexpectedEof).into()),
            Ok(n) => got += n,
            Err(err) if err.kind() == io::ErrorKind::Interrupted => {}
            Err(err) => return Err(err.into()),
        }
    }
    let length = u32::from_ne_bytes(length);
    if length > max {
        return Err(FrameError::TooLarge(length, max));
    }
    let mut message = vec![0u8; length as usize];
    input.read_exact(&mut message)?;
    Ok(Some(message))
}

/// Writes one message.
pub fn write_frame(output: &mut impl Write, message: &[u8]) -> io::Result<()> {
    let length = u32::try_from(message.len())
        .ok()
        .filter(|&n| n <= MAX_RESPONSE)
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the message is too large"))?;
    output.write_all(&length.to_ne_bytes())?;
    output.write_all(message)?;
    output.flush()
}

/// Writes a value as one message.
pub fn write_json(output: &mut impl Write, value: &impl Serialize) -> io::Result<()> {
    write_frame(
        output,
        &serde_json::to_vec(value).map_err(io::Error::other)?,
    )
}

/// Reads and checks one request: the schema, the protocol version and
/// every value (see the module's notes).
pub fn parse(message: &[u8]) -> Result<Request, String> {
    let request: Request =
        serde_json::from_slice(message).map_err(|e| format!("not a Zajil request: {e}"))?;
    check(&request)?;
    Ok(request)
}

fn check_version(version: u32) -> Result<(), String> {
    if version != PROTOCOL_VERSION {
        return Err(format!(
            "protocol version {version} is not supported (this Zajil speaks {PROTOCOL_VERSION})"
        ));
    }
    Ok(())
}

/// An instance or cookie store id: ASCII letters, digits, `-` and `_`.
fn check_id(id: &str, what: &str) -> Result<(), String> {
    if id.is_empty()
        || id.len() > MAX_ID
        || !id
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
    {
        return Err(format!(
            "the {what} id must be 1 to {MAX_ID} letters, digits, - or _"
        ));
    }
    Ok(())
}

fn check_cookies(cookies: &str) -> Result<(), String> {
    if cookies.len() > MAX_COOKIES {
        return Err(format!("the cookies are longer than {MAX_COOKIES} bytes"));
    }
    if !cookies.bytes().all(|b| (b' '..=b'~').contains(&b)) {
        return Err("the cookies have characters other than visible ASCII and spaces".into());
    }
    Ok(())
}

fn check(request: &Request) -> Result<(), String> {
    let version = match request {
        Request::Ping { protocol_version }
        | Request::ShowWindow { protocol_version }
        | Request::PrivateClosed { protocol_version }
        | Request::Download {
            protocol_version, ..
        }
        | Request::Link {
            protocol_version, ..
        }
        | Request::Cookies {
            protocol_version, ..
        }
        | Request::Video {
            protocol_version, ..
        } => *protocol_version,
    };
    check_version(version)?;
    match request {
        Request::Link { instance, .. } => check_id(instance, "instance")?,
        Request::Cookies {
            cookies: Some(cookies),
            ..
        } => check_cookies(cookies)?,
        Request::Video {
            page,
            media,
            cookies,
            private,
            ..
        } => {
            // A private window's cookies never go (review: the extension
            // never sends them; refused here too).
            if *private && cookies.is_some() {
                return Err("a private window's video comes without cookies".into());
            }
            check_url(page, "page")?;
            if let Some(media) = media {
                check_url(media, "video URL")?;
            }
            if let Some(cookies) = cookies {
                check_cookies(cookies)?;
            }
        }
        _ => {}
    }
    if let Request::Download {
        url,
        referrer,
        filename,
        cookies,
        instance,
        store,
        ticket,
        ..
    } = request
    {
        if let Some(instance) = instance {
            check_id(instance, "instance")?;
        }
        if let Some(ticket) = ticket {
            check_id(ticket, "ticket")?;
        }
        if let Some(store) = store {
            check_id(store, "cookie store")?;
        }
        check_url(url, "URL")?;
        if let Some(referrer) = referrer {
            check_url(referrer, "referrer")?;
        }
        if let Some(name) = filename {
            if name.len() > MAX_NAME {
                return Err(format!("the file name is longer than {MAX_NAME} bytes"));
            }
            if name.is_empty()
                || name
                    .chars()
                    .any(|c| c == '/' || c == '\\' || c.is_control())
            {
                return Err("the file name is empty, has a path, or has control characters".into());
            }
        }
        if let Some(cookies) = cookies {
            check_cookies(cookies)?;
        }
    }
    Ok(())
}

fn check_url(url: &str, what: &str) -> Result<(), String> {
    if url.len() > MAX_URL {
        return Err(format!("the {what} is longer than {MAX_URL} bytes"));
    }
    zajil_core::queue::check_url(url).map_err(|e| format!("the {what} is refused: {e}"))
}

/// The app's side of the channel: answers each connection with
/// `handle(request)`, until the process ends. An error on one connection,
/// or from `accept` (too many open files, a client gone before it was
/// accepted), never ends the loop; at most [`MAX_CONNECTIONS`] are served
/// at once, each given [`CHANNEL_TIMEOUT`] where the system allows it. A
/// [`Request::Link`] connection is the host's link to an extension: it is
/// handed to `links` and stays open, outside that count.
pub fn serve(
    listener: zajil_platform::ipc::Listener,
    handle: impl Fn(Request) -> Response + Send + Sync + 'static,
    links: Links,
) {
    let handle = Arc::new(handle);
    let busy = Arc::new(AtomicUsize::new(0));
    loop {
        let mut stream = match listener.accept() {
            Ok(stream) => stream,
            Err(err) => {
                eprintln!("zajil: the browser extension's channel: {err}");
                std::thread::sleep(Duration::from_millis(100));
                continue;
            }
        };
        if busy.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
            busy.fetch_sub(1, Ordering::SeqCst);
            let _ = write_json(&mut stream, &Response::refused("Zajil is busy; try again"));
            continue;
        }
        let (handle, busy, links) = (handle.clone(), busy.clone(), links.clone());
        std::thread::spawn(move || {
            let _ = stream.set_timeout(CHANNEL_TIMEOUT);
            if let Ok(Some(message)) = read_frame(&mut stream, MAX_REQUEST) {
                let answer = match parse(&message) {
                    Ok(Request::Link { instance, .. }) => {
                        busy.fetch_sub(1, Ordering::SeqCst);
                        links.serve(stream, instance);
                        return;
                    }
                    Ok(Request::Cookies { .. }) => {
                        Response::refused("cookies are answered on the link only")
                    }
                    Ok(request) => handle(request),
                    Err(why) => Response::refused(why),
                };
                let _ = write_json(&mut stream, &answer);
            }
            busy.fetch_sub(1, Ordering::SeqCst);
        });
    }
}

/// A question for one extension, and where its answer goes.
struct Question {
    asked: Asked,
    reply: mpsc::Sender<Option<Option<String>>>,
}

enum Asked {
    Cookies {
        url: String,
        store: Option<String>,
        ticket: String,
    },
    Keep(Vec<String>),
}

type Registry = HashMap<String, (u64, mpsc::Sender<Question>)>;
type OnLink = Arc<Mutex<Option<Box<dyn Fn(&str) + Send>>>>;

/// The app's links to the browsers' extensions (M5), one per extension
/// instance (a browser profile): Zajil asks the browser a download came
/// from for its cookies again after a restart. Each link answers one
/// question at a time (Windows' pipes take one call at a time).
#[derive(Clone, Default)]
pub struct Links {
    links: Arc<Mutex<Registry>>,
    next: Arc<AtomicU64>,
    /// Called with the instance when a link opens.
    on_link: OnLink,
}

impl Links {
    /// `f(instance)` runs each time an extension links (the app asks it
    /// at once for the downloads that wait for it).
    pub fn on_link(&self, f: impl Fn(&str) + Send + 'static) {
        *self.on_link.lock().unwrap_or_else(|e| e.into_inner()) = Some(Box::new(f));
    }

    /// Whether `instance` is linked now.
    pub fn is_linked(&self, instance: &str) -> bool {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains_key(instance)
    }

    /// Serves the link of `instance` on `stream` until it ends: the
    /// connection breaks, or a newer link of the same instance takes its
    /// place. Blocking.
    pub fn serve(&self, mut stream: zajil_platform::ipc::Stream, instance: String) {
        // The host answers every question within LINK_ANSWER.
        let _ = stream.set_timeout(LINK_ANSWER + Duration::from_secs(5));
        let (send, questions) = mpsc::channel::<Question>();
        let generation = self.next.fetch_add(1, Ordering::SeqCst);
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .insert(instance.clone(), (generation, send));
        if let Some(f) = &*self.on_link.lock().unwrap_or_else(|e| e.into_inner()) {
            f(&instance);
        }
        // Ends when this link is replaced (its sender dropped).
        while let Ok(question) = questions.recv() {
            let ask = self.next.fetch_add(1, Ordering::SeqCst);
            let wanted = match question.asked {
                Asked::Cookies { url, store, ticket } => ToExtension::CookiesWanted {
                    protocol_version: PROTOCOL_VERSION,
                    ask,
                    url,
                    store,
                    ticket,
                },
                Asked::Keep(tickets) => ToExtension::Keep {
                    protocol_version: PROTOCOL_VERSION,
                    ask,
                    tickets,
                },
            };
            let answer = write_json(&mut stream, &wanted)
                .ok()
                .and_then(|()| read_frame(&mut stream, MAX_REQUEST).ok().flatten());
            let Some(answer) = answer else {
                let _ = question.reply.send(None);
                break;
            };
            let _ = question.reply.send(match FromLink::parse(&answer) {
                Ok(FromLink::Cookies {
                    ask: answered,
                    cookies,
                    ..
                }) if answered == ask => Some(cookies),
                _ => None,
            });
        }
        let mut links = self.links.lock().unwrap_or_else(|e| e.into_inner());
        if links.get(&instance).is_some_and(|(g, _)| *g == generation) {
            links.remove(&instance);
        }
    }

    /// Asks extension `instance` for the cookies of `url` (from `store`)
    /// for the download with `ticket`: `Some(cookies)` when it answered
    /// (`None` cookies: no permission, no open download with that ticket,
    /// or none for that site), `None` when it is not linked, went away, or
    /// did not answer within `patience`. Blocking.
    pub fn ask(
        &self,
        instance: &str,
        url: &str,
        store: Option<&str>,
        ticket: &str,
        patience: Duration,
    ) -> Option<Option<String>> {
        let asked = Asked::Cookies {
            url: url.to_owned(),
            store: store.map(str::to_owned),
            ticket: ticket.to_owned(),
        };
        self.put(instance, asked, patience)
    }

    /// Tells extension `instance` which of its downloads' tickets are still
    /// open: it forgets the others. Whether it answered. Blocking.
    pub fn keep(&self, instance: &str, tickets: Vec<String>, patience: Duration) -> bool {
        self.put(instance, Asked::Keep(tickets), patience).is_some()
    }

    /// The instances linked now.
    pub fn instances(&self) -> Vec<String> {
        self.links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .keys()
            .cloned()
            .collect()
    }

    fn put(&self, instance: &str, asked: Asked, patience: Duration) -> Option<Option<String>> {
        let link = self
            .links
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(instance)
            .map(|(_, send)| send.clone())?;
        let (reply, answer) = mpsc::channel();
        link.send(Question { asked, reply }).ok()?;
        answer.recv_timeout(patience).ok().flatten()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn framed(json: &str) -> Vec<u8> {
        let mut out = Vec::new();
        write_frame(&mut out, json.as_bytes()).unwrap();
        out
    }

    #[test]
    fn a_frame_goes_and_comes_back() {
        let bytes = framed(r#"{"type":"ping","protocol_version":1}"#);
        let mut input = bytes.as_slice();
        let message = read_frame(&mut input, MAX_REQUEST).unwrap().unwrap();
        assert_eq!(
            parse(&message).unwrap(),
            Request::Ping {
                protocol_version: 1
            }
        );
        assert!(read_frame(&mut input, MAX_REQUEST).unwrap().is_none());
    }

    #[test]
    fn a_length_above_the_limit_is_refused_before_reading_on() {
        let mut bytes = (MAX_REQUEST + 1).to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        let err = read_frame(&mut bytes.as_slice(), MAX_REQUEST).unwrap_err();
        assert!(
            matches!(err, FrameError::TooLarge(n, _) if n == MAX_REQUEST + 1),
            "{err}"
        );
    }

    #[test]
    fn a_short_message_is_an_error() {
        let mut bytes = 10u32.to_ne_bytes().to_vec();
        bytes.extend_from_slice(b"{}");
        assert!(matches!(
            read_frame(&mut bytes.as_slice(), MAX_REQUEST),
            Err(FrameError::Io(_))
        ));
        assert!(matches!(
            read_frame(&mut &[1u8, 0][..], MAX_REQUEST),
            Err(FrameError::Io(_))
        ));
    }

    #[test]
    fn a_download_with_all_its_fields_is_read() {
        let request = parse(
            br#"{"type":"download","protocol_version":1,"url":"https://example.com/a.zip",
                "referrer":"https://example.com/page","filename":"a.zip","cookies":"sid=1; theme=dark"}"#,
        )
        .unwrap();
        assert_eq!(
            request,
            Request::Download {
                protocol_version: 1,
                url: "https://example.com/a.zip".into(),
                referrer: Some("https://example.com/page".into()),
                filename: Some("a.zip".into()),
                cookies: Some("sid=1; theme=dark".into()),
                instance: None,
                store: None,
                ticket: None,
                private: false,
            }
        );
    }

    #[test]
    fn what_the_schema_and_the_checks_refuse() {
        let long_url = format!("https://example.com/{}", "a".repeat(MAX_URL));
        let long_name = "a".repeat(MAX_NAME + 1);
        let long_cookies = "a".repeat(MAX_COOKIES + 1);
        for (message, why) in [
            ("not json", "not a Zajil request"),
            (r#"{"type":"ping"}"#, "missing field"),
            (
                r#"{"type":"ping","protocol_version":1,"extra":true}"#,
                "unknown field",
            ),
            (r#"{"type":"run","protocol_version":1}"#, "unknown variant"),
            (
                r#"{"type":"ping","protocol_version":2}"#,
                "protocol version 2",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"ftp://example.com/a"}"#,
                "URL is refused",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://u:p@example.com/a"}"#,
                "URL is refused",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://example.com/\u202ea"}"#,
                "URL is refused",
            ),
            (
                &format!(r#"{{"type":"download","protocol_version":1,"url":"{long_url}"}}"#),
                "longer than",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://example.com/a","referrer":"javascript:x"}"#,
                "referrer is refused",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://example.com/a","filename":"../a"}"#,
                "has a path",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://example.com/a","filename":"a\u0007"}"#,
                "control characters",
            ),
            (
                &format!(
                    r#"{{"type":"download","protocol_version":1,"url":"https://example.com/a","filename":"{long_name}"}}"#
                ),
                "longer than",
            ),
            (
                r#"{"type":"download","protocol_version":1,"url":"https://example.com/a","cookies":"a=1\r\nX-Evil: 1"}"#,
                "visible ASCII",
            ),
            (
                &format!(
                    r#"{{"type":"download","protocol_version":1,"url":"https://example.com/a","cookies":"{long_cookies}"}}"#
                ),
                "longer than",
            ),
        ] {
            let err = parse(message.as_bytes()).expect_err(message);
            assert!(err.contains(why), "{message}: {err}");
        }
    }

    #[test]
    fn a_video_is_read_with_its_page_and_checked_like_a_download() {
        assert_eq!(
            parse(
                br#"{"type":"video","protocol_version":1,"page":"https://example.com/watch","media":"https://cdn.example.com/v.mp4","cookies":"a=1"}"#
            )
            .unwrap(),
            Request::Video {
                protocol_version: 1,
                page: "https://example.com/watch".into(),
                media: Some("https://cdn.example.com/v.mp4".into()),
                cookies: Some("a=1".into()),
                private: false,
            }
        );
        assert_eq!(
            parse(br#"{"type":"video","protocol_version":1,"page":"https://example.com/"}"#)
                .unwrap(),
            Request::Video {
                protocol_version: 1,
                page: "https://example.com/".into(),
                media: None,
                cookies: None,
                private: false,
            }
        );
        for (message, why) in [
            (r#"{"type":"video","protocol_version":1}"#, "missing field"),
            (
                r#"{"type":"video","protocol_version":2,"page":"https://example.com/"}"#,
                "protocol version 2",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"file:///etc/passwd"}"#,
                "page is refused",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"https://example.com/","media":"blob:https://example.com/1"}"#,
                "video URL is refused",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"https://u:p@example.com/"}"#,
                "page is refused",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"https://example.com/","cookies":"a=1\nX: 1"}"#,
                "visible ASCII",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"https://example.com/","url":"https://example.com/"}"#,
                "unknown field",
            ),
            (
                r#"{"type":"video","protocol_version":1,"page":"https://example.com/","cookies":"a=1","private":true}"#,
                "without cookies",
            ),
        ] {
            let err = parse(message.as_bytes()).expect_err(message);
            assert!(err.contains(why), "{message}: {err}");
        }
    }

    #[test]
    fn answers_are_written_as_the_extension_reads_them() {
        let mut out = Vec::new();
        write_json(&mut out, &Response::refused("no")).unwrap();
        let message = read_frame(&mut out.as_slice(), MAX_RESPONSE)
            .unwrap()
            .unwrap();
        assert_eq!(
            std::str::from_utf8(&message).unwrap(),
            r#"{"protocol_version":1,"ok":false,"error":"no"}"#
        );
    }
}
