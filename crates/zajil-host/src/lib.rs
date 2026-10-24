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

use std::io::{self, Read, Write};

use serde::{Deserialize, Serialize};

/// The protocol's version: an extension and an app of different versions
/// say so clearly instead of misreading each other.
pub const PROTOCOL_VERSION: u32 = 1;
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

/// A message from the extension.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case", deny_unknown_fields)]
pub enum Request {
    /// Is the host there, and of which version?
    Ping { protocol_version: u32 },
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
    },
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
}

impl Response {
    pub fn ok() -> Self {
        Response {
            protocol_version: PROTOCOL_VERSION,
            ok: true,
            error: None,
        }
    }

    pub fn refused(why: impl Into<String>) -> Self {
        Response {
            protocol_version: PROTOCOL_VERSION,
            ok: false,
            error: Some(why.into()),
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

fn check(request: &Request) -> Result<(), String> {
    let version = match request {
        Request::Ping { protocol_version }
        | Request::Download {
            protocol_version, ..
        } => *protocol_version,
    };
    if version != PROTOCOL_VERSION {
        return Err(format!(
            "protocol version {version} is not supported (this Zajil speaks {PROTOCOL_VERSION})"
        ));
    }
    if let Request::Download {
        url,
        referrer,
        filename,
        cookies,
        ..
    } = request
    {
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
            if cookies.len() > MAX_COOKIES {
                return Err(format!("the cookies are longer than {MAX_COOKIES} bytes"));
            }
            if !cookies.bytes().all(|b| (b' '..=b'~').contains(&b)) {
                return Err(
                    "the cookies have characters other than visible ASCII and spaces".into(),
                );
            }
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
