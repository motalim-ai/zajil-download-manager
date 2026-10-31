//! yt-dlp's output: read within bounds, line by line, and its progress.

use std::io::{BufRead, BufReader, Read};
use std::path::PathBuf;

use super::VideoEvent;
use super::args::PROGRESS_PREFIX;
use super::messages::{says_challenge, shown_message};

/// The longest JSON a probe may print (a long playlist, or a page with
/// many formats), and the longest line of anything else.
const MAX_JSON: u64 = 64 * 1024 * 1024;
const MAX_LINE: usize = 64 * 1024;
/// The most entries of a playlist that are listed.
pub const MAX_PLAYLIST_ENTRIES: usize = 1000;

pub(super) struct ReadOutput {
    pub(super) json: Vec<u8>,
    pub(super) final_path: Option<PathBuf>,
    pub(super) error: Option<String>,
    /// [`Output::loaded`].
    pub(super) loaded: bool,
}

/// What yt-dlp's verbose header says once it has loaded its request
/// handlers (and with them its libraries).
const LOADED_LINE: &str = "[debug] Request Handlers";

/// Reads yt-dlp's output until it closes: the JSON of a probe (bounded),
/// or the progress and final path of a download; and the last error line.
pub(super) fn read_output(
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    is_download: bool,
    events: tokio::sync::mpsc::UnboundedSender<VideoEvent>,
) -> ReadOutput {
    let errors = std::thread::spawn(move || {
        let mut last = None;
        // A warning that says why, when the error names only what followed
        // (no format left after YouTube's challenge went unsolved).
        let mut cause = None;
        let mut loaded = false;
        for line in bounded_lines(stderr) {
            loaded |= line.starts_with(LOADED_LINE);
            if let Some(rest) = line.strip_prefix("ERROR:") {
                last = Some(shown_message(rest.trim()));
            } else if let Some(rest) = line.strip_prefix("WARNING:")
                && cause.is_none()
                && says_challenge(rest)
            {
                cause = Some(shown_message(rest.trim()));
            }
        }
        let error = match (cause, last) {
            (Some(cause), Some(last)) => Some(format!("{cause} / {last}")),
            (cause, last) => last.or(cause),
        };
        (error, loaded)
    });
    let mut json = Vec::new();
    let mut final_path = None;
    if is_download {
        for line in bounded_lines(stdout) {
            if let Some(rest) = line.strip_prefix(PROGRESS_PREFIX) {
                if let Some(event) = parse_progress(rest) {
                    let _ = events.send(event);
                }
            } else if !line.trim().is_empty() {
                final_path = Some(PathBuf::from(line.trim_end_matches(['\r', '\n'])));
            }
        }
    } else {
        let _ = stdout.take(MAX_JSON).read_to_end(&mut json);
    }
    let (error, loaded) = errors.join().unwrap_or((None, false));
    ReadOutput {
        json,
        final_path,
        error,
        loaded,
    }
}

/// Lines of at most [`MAX_LINE`] bytes, lossy UTF-8; a longer line is cut.
fn bounded_lines(from: impl Read) -> impl Iterator<Item = String> {
    let mut reader = BufReader::new(from);
    std::iter::from_fn(move || {
        let mut buf = Vec::new();
        match (&mut reader)
            .take(MAX_LINE as u64)
            .read_until(b'\n', &mut buf)
        {
            Ok(0) | Err(_) => None,
            Ok(_) => {
                if !buf.ends_with(b"\n") {
                    // Cut: skip the rest of this line, keeping nothing of it
                    // (review of the video runner: it was read whole).
                    while let Ok(chunk) = reader.fill_buf() {
                        if chunk.is_empty() {
                            break;
                        }
                        match chunk.iter().position(|&b| b == b'\n') {
                            Some(at) => {
                                reader.consume(at + 1);
                                break;
                            }
                            None => {
                                let n = chunk.len();
                                reader.consume(n);
                            }
                        }
                    }
                }
                Some(String::from_utf8_lossy(&buf).into_owned())
            }
        }
    })
}

/// A number yt-dlp printed (`NA` when unknown), as bytes.
fn number(word: Option<&str>) -> Option<u64> {
    let value: f64 = word?.parse().ok()?;
    (value.is_finite() && value >= 0.0).then(|| value.min(u64::MAX as f64) as u64)
}

fn parse_progress(rest: &str) -> Option<VideoEvent> {
    let mut words = rest.split_whitespace();
    let done = number(words.next())?;
    let total = number(words.next());
    let estimate = number(words.next());
    let speed = number(words.next());
    Some(VideoEvent::Progress {
        done,
        total: total.or(estimate),
        speed,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;

    #[test]
    fn progress_lines_with_unknown_numbers() {
        assert_eq!(
            parse_progress(" 1024 NA 4096.0 512.5"),
            Some(VideoEvent::Progress {
                done: 1024,
                total: Some(4096),
                speed: Some(512)
            })
        );
        assert_eq!(parse_progress(" NA NA NA NA"), None);
        assert_eq!(
            parse_progress(" 1e30 NA NA NA"),
            Some(VideoEvent::Progress {
                done: u64::MAX,
                total: None,
                speed: None
            })
        );
    }

    /// AGENTS.md §10.8: a line longer than the limit is cut, and the rest
    /// of it is skipped without being kept (it was read whole).
    #[test]
    fn a_huge_line_is_cut_and_its_rest_is_never_kept() {
        struct Endless {
            left: u64,
            then: &'static [u8],
        }
        impl Read for Endless {
            fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
                if self.left > 0 {
                    let n = (buf.len() as u64).min(self.left) as usize;
                    buf[..n].fill(b'a');
                    self.left -= n as u64;
                    return Ok(n);
                }
                let n = buf.len().min(self.then.len());
                buf[..n].copy_from_slice(&self.then[..n]);
                self.then = &self.then[n..];
                Ok(n)
            }
        }
        let lines: Vec<String> = bounded_lines(Endless {
            left: 100 * 1024 * 1024,
            then: b"\nERROR: next\n",
        })
        .collect();
        assert_eq!(lines.len(), 2);
        assert_eq!(lines[0].len(), MAX_LINE);
        assert_eq!(lines[1], "ERROR: next\n");
    }
}
