//! The runner of yt-dlp for Zajil: its tools, the checks before a run, and
//! the probe of a page.

use std::io;
use std::path::Path;
#[cfg(feature = "test-support")]
use std::path::PathBuf;

use crate::engine::{Engine, Stop};
use crate::tools::{Tool, Toolbox};

use super::args::{Arg, WARM_UP_TIME, WARM_UP_URL, base_args};
use super::files::checked;
use super::js::{Checked, JsRuntime, JsSandbox};
use super::messages::{MAX_PLAYLIST, ToolProblem, hls_is_protected, says_no_format, shown_message};
use super::output::MAX_PLAYLIST_ENTRIES;
use super::runs::Runs;
use super::{Choice, Format, Hidden, Probed, Raw, Sent, Video, VideoError, VideoEvent};

/// A download's progress as a whole. yt-dlp counts each part from 0 (a
/// merge: the picture, then the sound), so the parts before are added;
/// the whole is the chosen formats' sizes when the page gave them all.
pub(super) struct Overall {
    /// The parts that ended.
    before: u64,
    done: u64,
    total: Option<u64>,
    expected: Option<u64>,
}

impl Overall {
    pub(super) fn new(expected: Option<u64>) -> Overall {
        Overall {
            before: 0,
            done: 0,
            total: None,
            expected,
        }
    }

    pub(super) fn of(&mut self, event: VideoEvent) -> VideoEvent {
        let VideoEvent::Progress { done, total, speed } = event else {
            return event;
        };
        if done < self.done {
            // Back to the start: the part before has ended.
            self.before += self.total.unwrap_or(0).max(self.done);
        }
        self.done = done;
        self.total = total;
        let so_far = self.before + done;
        let known = total.map(|t| self.before + t.max(done));
        VideoEvent::Progress {
            done: so_far,
            total: match (self.expected, known) {
                (Some(expected), known) => Some(expected.max(known.unwrap_or(so_far))),
                (None, known) => known,
            },
            speed,
        }
    }
}

/// The size of what `choice` downloads, when the page gave every part's.
pub(super) fn expected_size(choice: &Choice, formats: &[Format]) -> Option<u64> {
    let size = |id: &str| {
        formats
            .iter()
            .find(|f| f.id == id)
            .and_then(|f| f.filesize.or(f.filesize_approx))
    };
    match choice {
        Choice::Format(id) => size(id),
        Choice::Merge { video, audio } => Some(size(video)? + size(audio)?),
        Choice::AudioOnly => None,
    }
}

/// One video download. Its `Debug` shows neither its link nor its cookies.
#[derive(Clone, Copy)]
pub struct Download<'a> {
    pub url: &'a str,
    pub choice: &'a Choice,
    /// The formats the probe found: the HLS ones the choice may use are
    /// checked for DRM before yt-dlp runs.
    pub formats: &'a [Format],
    /// The link's own `Cookie` header from the browser, if any.
    pub cookies: Option<&'a str>,
    /// The folder the video goes to.
    pub dest: &'a Path,
    /// Its name, without extension (the page's title, usually).
    pub name: &'a str,
}

impl std::fmt::Debug for Download<'_> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Download")
            .field("url", &crate::queue::shown_url(self.url))
            .field("choice", self.choice)
            .field("formats", &self.formats.len())
            .field("cookies", &self.cookies.map(|_| "…"))
            .field("dest", &self.dest)
            .field("name", &self.name)
            .finish()
    }
}

/// Runs yt-dlp for Zajil.
pub struct VideoRunner {
    pub(super) tools: Toolbox,
    pub(super) runs: Runs,
    /// For Zajil's own requests (the HLS playlists of the DRM check).
    pub(super) engine: Engine,
    /// Tests only: a PEM file of the test authority that yt-dlp trusts.
    #[cfg(feature = "test-support")]
    test_root: Option<PathBuf>,
    /// The JavaScript runtime for YouTube's challenges ([`find_js_runtime`]),
    /// or none: then YouTube may fail, and other sites do not need one.
    pub(super) js: Option<JsRuntime>,
    /// The weakest sandbox `zajil-jsbox` reported so far.
    pub(super) js_sandbox: std::sync::Mutex<Option<JsSandbox>>,
}

impl VideoRunner {
    /// A runner of the tools in `tools`, each checked again before every
    /// run.
    pub fn new(tools: Toolbox, runs: Runs, engine: Engine) -> Self {
        VideoRunner {
            tools,
            runs,
            engine,
            #[cfg(feature = "test-support")]
            test_root: None,
            js: None,
            js_sandbox: std::sync::Mutex::new(None),
        }
    }

    /// Gives yt-dlp `js` for YouTube's challenges ([`find_js_runtime`]).
    /// With our QuickJS-NG, `qjs` is checked before every run like the
    /// other tools.
    pub fn with_js(mut self, js: Option<JsRuntime>) -> Self {
        self.js = js;
        self
    }

    /// What the sandbox of `zajil-jsbox` held in the runs so far (the
    /// weakest), if it ran: for the log, and to tell the user when it is
    /// not everything (owner's condition: QuickJS-NG runs without it where
    /// the system has none, but that is said).
    pub fn js_sandbox(&self) -> Option<JsSandbox> {
        *self.js_sandbox.lock().unwrap_or_else(|e| e.into_inner())
    }

    /// Lets yt-dlp trust the tests' temporary authority (a PEM file), like
    /// [`Engine::with_test_root`]: only with the `test-support` feature,
    /// which no shipped binary enables (checked in CI).
    #[cfg(feature = "test-support")]
    pub fn with_test_root(mut self, ca_pem: PathBuf) -> Self {
        self.test_root = Some(ca_pem);
        self
    }

    /// The run's arguments and the environment it adds: nothing but in
    /// tests that trust their own authority.
    #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
    pub(super) fn all_args(
        &self,
        mut args: Vec<Arg>,
    ) -> (Vec<Arg>, Vec<(String, std::ffi::OsString)>) {
        #[cfg_attr(not(feature = "test-support"), allow(unused_mut))]
        let mut env = Vec::new();
        #[cfg(feature = "test-support")]
        if let Some(ca) = &self.test_root {
            args.push(Arg::TestNoCertifi);
            env.push(("SSL_CERT_FILE".to_owned(), ca.clone().into_os_string()));
        }
        (args, env)
    }

    /// yt-dlp, ffmpeg and ffprobe, and `qjs` when it is the runtime,
    /// checked now.
    pub(super) async fn checked_tools(&self) -> Result<Checked, VideoError> {
        let qjs = match &self.js {
            Some(JsRuntime::QuickJs { .. }) => Some((
                self.tools.verified(Tool::QuickJs).await?,
                self.tools.sha256(Tool::QuickJs)?,
            )),
            _ => None,
        };
        Ok(Checked {
            yt_dlp: self.tools.verified(Tool::YtDlp).await?,
            ffmpeg: self.tools.verified(Tool::Ffmpeg).await?,
            _ffprobe: self.tools.verified(Tool::Ffprobe).await?,
            qjs,
        })
    }

    /// Refuses `choice` if an HLS format it may use is protected in a way
    /// yt-dlp would not catch (it would hand the playlist to ffmpeg).
    pub(super) async fn check_hls(
        &self,
        request: &Download<'_>,
        stop: &Stop,
    ) -> Result<(), VideoError> {
        let used = |f: &Format| match request.choice {
            Choice::Format(id) => f.id == *id,
            Choice::Merge { video, audio } => f.id == *video || f.id == *audio,
            Choice::AudioOnly => f.has_audio() || f.acodec.is_none(),
        };
        // Never skipped: a choice the probe's formats do not hold could be
        // a protected one nobody checked (review of the video runner).
        let known = |id: &String| request.formats.iter().any(|f| f.id == *id);
        let all_known = match request.choice {
            Choice::Format(id) => known(id),
            Choice::Merge { video, audio } => known(video) && known(audio),
            Choice::AudioOnly => !request.formats.is_empty(),
        };
        if !all_known {
            return Err(VideoError::Refused(
                "the chosen format is not one the probe found".into(),
            ));
        }
        let engine = self.engine.with_browser(crate::browser::FromBrowser::new(
            request.url,
            request.cookies,
            None,
        ));
        for format in request.formats.iter().filter(|f| f.is_hls() && used(f)) {
            let Some(Hidden(playlist_url)) = &format.url else {
                continue;
            };
            let playlist = engine
                .fetch_text(playlist_url, MAX_PLAYLIST, stop)
                .await
                .map_err(|e| match e {
                    crate::engine::Error::Stopped { .. } => VideoError::Stopped,
                    e => VideoError::Tool(format!(
                        "the HLS playlist could not be read: {}",
                        shown_message(&e.to_string())
                    )),
                })?;
            if hls_is_protected(&playlist) {
                return Err(VideoError::Drm);
            }
        }
        Ok(())
    }

    /// Starts yt-dlp once without reaching the network (owner's decision,
    /// 2026-10-27): the first start of a newly unpacked yt-dlp is slow, as
    /// the system checks each of its files the first time, so it happens
    /// while Zajil says it is preparing the tools, not at the first video.
    /// The run itself fails (it has no network) and that is fine, once
    /// yt-dlp has said which request handlers it loaded. Without that, or
    /// after [`WARM_UP_TIME`], it is [`VideoError::NotReady`]; a tool that
    /// cannot be checked is [`VideoError::Tools`], one that cannot be
    /// started [`VideoError::Io`], and a stop [`VideoError::Stopped`].
    pub async fn warm_up(&self, stop: &Stop) -> Result<(), VideoError> {
        let url = checked(WARM_UP_URL)?;
        // Every request of the run comes here, and is dropped at once: a
        // port of our own, not one another program could listen on
        // (review of #278).
        let proxy = tokio::net::TcpListener::bind((std::net::Ipv4Addr::LOCALHOST, 0)).await?;
        let port = proxy.local_addr()?.port();
        let dropping = tokio::spawn(async move {
            while let Ok((connection, _)) = proxy.accept().await {
                drop(connection);
            }
        });
        let mut args = base_args();
        args.push(Arg::WarmUp(port));
        let mut ignore = |_| {};
        let run = self.run_unjudged(&url, args, None, None, stop, &mut ignore);
        let ran = tokio::time::timeout(WARM_UP_TIME, run).await;
        dropping.abort();
        match ran {
            // Its request fails (no network), and that is fine; it worked
            // only if yt-dlp said which request handlers it loaded (review
            // of #278: one that crashed while loading Python counted as
            // done).
            Ok(Ok(output)) if output.loaded => Ok(()),
            Ok(Ok(output)) => {
                Err(VideoError::NotReady(output.error.unwrap_or_else(|| {
                    "it ended before loading what it needs".into()
                })))
            }
            // Dropped: the run's tree is ended and its folders removed.
            Err(_elapsed) => Err(VideoError::NotReady(format!(
                "it did not finish within {} s",
                WARM_UP_TIME.as_secs()
            ))),
            Ok(Err(err)) => Err(err),
        }
    }

    /// What `url` holds: one video and its formats, or a playlist's items.
    /// Downloads nothing. `cookies` is the link's own `Cookie` header from
    /// the browser, if any (ADR-0005).
    pub async fn probe(
        &self,
        url: &str,
        cookies: Option<&str>,
        stop: &Stop,
    ) -> Result<Probed, VideoError> {
        let url = checked(url)?;
        let mut args = base_args();
        args.extend([
            Arg::NoPlaylist,
            Arg::FlatPlaylist,
            Arg::PlaylistEnd,
            Arg::DumpSingleJson,
        ]);
        let output = self
            .run(&url, args, cookies, None, stop, &mut |_| {})
            .await?;
        // Up to 64 MiB: read on a blocking thread.
        let stdout = output.stdout;
        let raw: Raw = tokio::task::spawn_blocking(move || serde_json::from_slice(&stdout))
            .await
            .map_err(io::Error::other)?
            .map_err(|e| VideoError::Output(format!("unreadable JSON ({e})")))?;
        if raw.kind.as_deref() == Some("playlist") {
            let mut entries = raw.entries;
            entries.truncate(MAX_PLAYLIST_ENTRIES);
            for entry in &mut entries {
                // A playlist yt-dlp opened (a channel's page) may give
                // only its page.
                if entry.url.is_none() {
                    entry.url = entry.webpage_url.take();
                }
            }
            return Ok(Probed::Playlist {
                title: raw.title,
                entries,
            });
        }
        if !raw.formats.is_empty() && raw.formats.iter().all(Format::is_protected) {
            return Err(VideoError::Drm);
        }
        Ok(Probed::Video(Video {
            title: raw.title,
            duration: raw.duration,
            formats: raw
                .formats
                .into_iter()
                .filter(|f| !f.is_protected())
                .collect(),
        }))
    }

    /// [`probe`](Self::probe) with the browser's cookies, and once more
    /// without them when that found no format at all. Signed in to YouTube,
    /// yt-dlp asks only the players that need JavaScript; with no runtime
    /// for it only pictures are left, while the player it asks without
    /// cookies needs none. Also says whether the cookies were kept: the
    /// download must go the same way, or it would meet the same wall.
    pub async fn probe_with_or_without_cookies(
        &self,
        url: &str,
        cookies: Option<&str>,
        stop: &Stop,
    ) -> Result<(Probed, bool), VideoError> {
        match self.probe(url, cookies, stop).await {
            Err(VideoError::Tool(message)) if cookies.is_some() && says_no_format(&message) => {
                Ok((self.probe(url, None, stop).await?, false))
            }
            probed => Ok((probed?, cookies.is_some())),
        }
    }

    /// What the browser sent holds: its page asked as
    /// [`probe_with_or_without_cookies`](Self::probe_with_or_without_cookies)
    /// does; and when yt-dlp cannot read the page at all (a site it does not
    /// know), the video's own link the browser saw, `media`. That one is
    /// asked without the page's cookies: they are the page's alone
    /// (ADR-0005).
    pub async fn probe_sent(
        &self,
        page: &str,
        media: Option<&str>,
        cookies: Option<&str>,
        stop: &Stop,
    ) -> Result<Sent, VideoError> {
        let error = match self
            .probe_with_or_without_cookies(page, cookies, stop)
            .await
        {
            Ok((probed, cookies_kept)) => {
                return Ok(Sent {
                    probed,
                    url: page.to_owned(),
                    cookies_kept,
                });
            }
            Err(error) => error,
        };
        let unreadable = matches!(error, VideoError::Output(_))
            || matches!(error, VideoError::Tool(_))
                && matches!(error.problem(), None | Some(ToolProblem::Unsupported));
        match media.filter(|media| unreadable && *media != page) {
            Some(media) => match self.probe(media, None, stop).await {
                Ok(probed) => Ok(Sent {
                    probed,
                    url: media.to_owned(),
                    cookies_kept: false,
                }),
                // The page's own failure says more.
                Err(VideoError::Stopped) => Err(VideoError::Stopped),
                Err(_) => Err(error),
            },
            None => Err(error),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_download_s_debug_shows_no_link_secret_nor_cookie() {
        let choice = Choice::Format("18".into());
        let request = Download {
            url: "https://example.com/v?token=SECRET",
            choice: &choice,
            formats: &[],
            cookies: Some("sid=SECRET"),
            dest: Path::new("/tmp"),
            name: "x",
        };
        let shown = format!("{request:?}");
        assert!(!shown.contains("SECRET"), "{shown}");
    }
}
