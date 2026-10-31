//! Video downloads with yt-dlp and our ffmpeg (ADR-0009, sections 4 to 6).
//!
//! Every run of yt-dlp:
//! - starts the checked tool directly, never through a shell, as a process
//!   tree that ends with it and with Zajil ([`zajil_platform::process`]);
//! - gets only arguments built here from a fixed set ([`Arg`]): the page's
//!   URL is checked and comes last, alone, after `--`; nothing from the page
//!   ever takes the place of an option, a path or an output template;
//! - ignores the user's yt-dlp configuration and plugins, fetches no code,
//!   uses yt-dlp's own downloader, and finds only our ffmpeg;
//! - runs only tools checked right before it ([`Toolbox::verified`]: the
//!   pinned files, in folders no other user can change), kept open until
//!   its whole tree has ended;
//! - has a private folder of its own under `<data dir>/run` (its temporary
//!   files, the cookie file when there is one, and a record of its process
//!   tree), removed as soon as the tree has ended; what a crash leaves is
//!   swept by [`Runs::clean_leftovers`], which the program calls at its
//!   start;
//! - writes the video into a folder of its own next to where it goes,
//!   under a fixed name; Zajil names the final file itself.
//!
//! Content protected by DRM is never downloaded (AGENTS.md, section 6).

use std::io;

use serde::Deserialize;

mod args;
mod download;
mod fast;
mod files;
mod js;
mod messages;
mod output;
mod runner;
mod runs;

use args::format_spec;
pub use args::{Arg, check_format_id, command_line};
pub use fast::Saved;
// `fast.rs` reaches these as its parent's.
use download::Finish;
use files::without_media_extension;
pub use files::{kept_folder, move_video, part_files, remove_kept};
pub use js::{JsRuntime, JsSandbox, find_js_runtime, jsbox_beside};
pub use messages::{ToolProblem, hls_is_protected, shown_message};
pub use output::MAX_PLAYLIST_ENTRIES;
pub use runner::{Download, VideoRunner};
use runs::random_hex;
pub use runs::{Runs, netscape_cookies};

/// What to download.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Choice {
    /// One format with both picture and sound, or audio alone.
    Format(String),
    /// A picture and a sound format, merged by ffmpeg without re-encoding.
    Merge { video: String, audio: String },
    /// Sound only, as m4a: the best m4a the site offers, else the best
    /// sound turned into AAC by our ffmpeg.
    AudioOnly,
}

impl Choice {
    /// The format ids it names (none for `AudioOnly`, chosen as it runs).
    pub fn ids(&self) -> Vec<&str> {
        match self {
            Choice::Format(id) => vec![id],
            Choice::Merge { video, audio } => vec![video, audio],
            Choice::AudioOnly => Vec::new(),
        }
    }

    fn args(&self) -> Result<Vec<Arg>, VideoError> {
        Ok(match self {
            Choice::Format(id) => vec![Arg::Format(format_spec(id)?)],
            Choice::Merge { video, audio } => vec![Arg::Format(format!(
                "{}+{}",
                format_spec(video)?,
                format_spec(audio)?
            ))],
            Choice::AudioOnly => vec![
                Arg::Format("bestaudio[ext=m4a]/bestaudio".to_owned()),
                Arg::AudioOnlyM4a,
            ],
        })
    }
}

/// Why a video cannot be downloaded.
#[derive(Debug, thiserror::Error)]
pub enum VideoError {
    #[error("{0}")]
    Refused(String),
    /// Protected by DRM: never downloaded.
    #[error("this video is protected (DRM), and Zajil does not download protected content")]
    Drm,
    /// What yt-dlp said, without the secrets a URL may carry.
    #[error("yt-dlp: {0}")]
    Tool(String),
    #[error("yt-dlp's output was not what Zajil expects: {0}")]
    Output(String),
    #[error("stopped")]
    Stopped,
    /// yt-dlp's first run after its download ([`VideoRunner::warm_up`])
    /// did not get as far as loading what it needs.
    #[error("yt-dlp could not start: {0}")]
    NotReady(String),
    /// A tool is missing, or is not the checked file (ADR-0009).
    #[error(transparent)]
    Tools(#[from] crate::tools::ToolError),
    #[error(transparent)]
    Io(#[from] io::Error),
}

/// A format a page offers, as the probe found it.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Format {
    #[serde(rename = "format_id")]
    pub id: String,
    #[serde(default)]
    pub ext: String,
    #[serde(default)]
    pub width: Option<u32>,
    #[serde(default)]
    pub height: Option<u32>,
    #[serde(default)]
    pub fps: Option<f64>,
    /// `none` for a format without picture.
    #[serde(default)]
    pub vcodec: Option<String>,
    /// `none` for a format without sound.
    #[serde(default)]
    pub acodec: Option<String>,
    #[serde(default)]
    pub filesize: Option<u64>,
    #[serde(default)]
    pub filesize_approx: Option<u64>,
    /// Total bitrate in kbit/s.
    #[serde(default)]
    pub tbr: Option<f64>,
    #[serde(default)]
    pub has_drm: Option<serde_json::Value>,
    /// How yt-dlp gets it (`https`, `m3u8_native`, `http_dash_segments`…).
    #[serde(default)]
    pub protocol: Option<String>,
    /// Its URL (for an HLS format, its playlist), kept in memory only for
    /// Zajil's own DRM check: it may carry a token, so it is never shown.
    #[serde(default)]
    url: Option<Hidden>,
    /// The program its link was made for, of its `http_headers`: the only
    /// one of them read (the rest may hold secrets). YouTube refuses some
    /// of its apps' links to another program (owner's report, 2026-10-30:
    /// 403, then yt-dlp from the start).
    #[serde(default, rename = "http_headers", deserialize_with = "user_agent_of")]
    agent: Option<String>,
    /// A sound's language, and how much yt-dlp prefers it: YouTube offers
    /// a video's own sound beside sounds dubbed by AI, in the same
    /// formats, and marks its own `10` (and «original» in its note).
    #[serde(default)]
    pub language: Option<String>,
    #[serde(default)]
    language_preference: Option<i64>,
    #[serde(default)]
    format_note: Option<String>,
}

/// The `User-Agent` of a format's `http_headers`, and nothing else.
fn user_agent_of<'de, D: serde::Deserializer<'de>>(d: D) -> Result<Option<String>, D::Error> {
    #[derive(Deserialize)]
    struct Headers {
        #[serde(rename = "User-Agent")]
        agent: Option<String>,
    }
    Ok(Option::<Headers>::deserialize(d)?.and_then(|h| h.agent))
}

/// A value that may hold a secret: printed as `(hidden)`.
#[derive(Clone, PartialEq, Deserialize)]
#[serde(transparent)]
struct Hidden(String);

impl std::fmt::Debug for Hidden {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str("(hidden)")
    }
}

impl Format {
    /// How much its sound is the video's own: higher for the original, as
    /// yt-dlp ranks it, lower for a dubbed one (users' reports, 2026-10-31:
    /// YouTube videos came with an AI dub instead of their own voice).
    pub fn sound_rank(&self) -> i64 {
        let note = self
            .format_note
            .as_deref()
            .unwrap_or("")
            .to_ascii_lowercase();
        let marked = if note.contains("original") {
            100
        } else if note.contains("dubbed") {
            -100
        } else {
            0
        };
        marked + self.language_preference.unwrap_or(0)
    }

    /// The program its link was made for (its `User-Agent`), if it says.
    pub fn user_agent(&self) -> Option<&str> {
        self.agent.as_deref()
    }

    pub fn has_video(&self) -> bool {
        self.vcodec.as_deref().is_some_and(|c| c != "none")
    }

    pub fn has_audio(&self) -> bool {
        self.acodec.as_deref().is_some_and(|c| c != "none")
    }

    /// Whether yt-dlp gets it through an HLS playlist.
    pub fn is_hls(&self) -> bool {
        self.protocol
            .as_deref()
            .is_some_and(|p| p.starts_with("m3u8"))
    }

    /// yt-dlp marks protected formats with `has_drm` true (or `"maybe"`).
    pub fn is_protected(&self) -> bool {
        match &self.has_drm {
            Some(serde_json::Value::Bool(b)) => *b,
            Some(serde_json::Value::String(s)) => !s.is_empty(),
            _ => false,
        }
    }
}

/// A single video.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Video {
    #[serde(default)]
    pub title: String,
    #[serde(default)]
    pub duration: Option<f64>,
    #[serde(default)]
    pub formats: Vec<Format>,
}

impl Video {
    /// The best choice when the user names none: the highest picture
    /// with sound in one format, or the highest picture-only format with
    /// the best sound-only one when that picture is higher; sound alone
    /// for a page that has no picture. `None` when nothing is usable. A
    /// direct file comes before an HLS playlist of the same kind.
    ///
    /// An MP4 that every player opens (H.264 picture, AAC sound) comes
    /// first when the page has one: VP9, AV1 or Opus would be merged into
    /// MKV or WebM, which QuickTime on macOS does not play.
    pub fn best_choice(&self) -> Option<Choice> {
        let height = |f: &Format| f.height.unwrap_or(0);
        let rate = |f: &Format| f.tbr.unwrap_or(0.0);
        let by_rate = |a: &&Format, b: &&Format| {
            rate(a)
                .partial_cmp(&rate(b))
                .unwrap_or(std::cmp::Ordering::Equal)
        };
        let better = |a: &&Format, b: &&Format| height(a).cmp(&height(b)).then(by_rate(a, b));
        let mp4_picture = |f: &Format| {
            f.vcodec
                .as_deref()
                .is_some_and(|c| c.starts_with("avc1") || c.starts_with("h264"))
        };
        let mp4_sound = |f: &Format| {
            f.acodec.as_deref().is_some_and(|c| c.starts_with("mp4a")) || f.ext == "m4a"
        };
        // The best of `kind`: a direct file before an HLS playlist, then
        // among those `preferred` if any. Zajil's own engine downloads a
        // direct file with several connections and resumes it; an HLS
        // playlist goes piece by piece through yt-dlp and starts over
        // (owner's report, 2026-10-29: a playlist's YouTube videos went
        // as itag 96, slowly).
        let top = |kind: &dyn Fn(&Format) -> bool,
                   preferred: &dyn Fn(&Format) -> bool,
                   order: &dyn Fn(&&Format, &&Format) -> std::cmp::Ordering| {
            let direct = self.formats.iter().any(|f| kind(f) && !f.is_hls());
            let of_kind = move || {
                self.formats
                    .iter()
                    .filter(move |f| kind(f) && !(direct && f.is_hls()))
            };
            of_kind()
                .filter(|f| preferred(f))
                .max_by(|a, b| order(a, b))
                .or_else(|| of_kind().max_by(|a, b| order(a, b)))
        };
        let both = top(
            &|f| f.has_video() && f.has_audio(),
            &|f| mp4_picture(f) && mp4_sound(f),
            &better,
        );
        let picture = top(&|f| f.has_video() && !f.has_audio(), &mp4_picture, &better);
        // The highest MP4 file whose codecs the site does not say.
        let plain_mp4 = self
            .formats
            .iter()
            .filter(|f| f.vcodec.is_none() && f.acodec.is_none() && f.ext == "mp4" && !f.is_hls())
            .max_by(&better);
        // The video's own sound before a dubbed one, then the best rate.
        let by_sound =
            |a: &&Format, b: &&Format| a.sound_rank().cmp(&b.sound_rank()).then(by_rate(a, b));
        let sound = top(&|f| f.has_audio() && !f.has_video(), &mp4_sound, &by_sound);
        match (both, picture, sound) {
            // A higher picture is merged in, unless it would trade an MP4
            // every player opens for one QuickTime cannot play (owner's
            // report, 2026-10-30: Instagram's 1440p VP9 played as sound
            // only, beside its H.264 with sound).
            (Some(both), Some(picture), Some(sound))
                if height(picture) > height(both)
                    && (mp4_picture(picture) || !(mp4_picture(both) && mp4_sound(both))) =>
            {
                Some(Choice::Merge {
                    video: picture.id.clone(),
                    audio: sound.id.clone(),
                })
            }
            (Some(both), _, _) => Some(Choice::Format(both.id.clone())),
            // Only a VP9 or AV1 picture to merge, beside the site's own MP4
            // file whose codecs it does not say (Facebook's and Instagram's
            // are H.264 with their sound): that file, which plays anywhere.
            (None, Some(picture), Some(_)) if !mp4_picture(picture) && plain_mp4.is_some() => {
                plain_mp4.map(|f| Choice::Format(f.id.clone()))
            }
            (None, Some(picture), Some(sound)) => Some(Choice::Merge {
                video: picture.id.clone(),
                audio: sound.id.clone(),
            }),
            (None, None, Some(_)) => Some(Choice::AudioOnly),
            // Formats whose codecs the page does not say (a direct file):
            // the last one, yt-dlp lists the best last.
            _ => self.formats.last().map(|f| Choice::Format(f.id.clone())),
        }
    }
}

/// One item of a playlist, to be probed and downloaded on its own; or a
/// playlist inside it (a channel's «Playlists»), to be opened.
#[derive(Debug, Clone, PartialEq, Deserialize)]
pub struct Entry {
    #[serde(default)]
    pub url: Option<String>,
    #[serde(default)]
    pub title: Option<String>,
    /// `playlist`, or `url` for an item yt-dlp did not open.
    #[serde(rename = "_type", default)]
    pub kind: Option<String>,
    /// What reads an item yt-dlp did not open: `YoutubeTab` for a
    /// playlist or a channel's tab.
    #[serde(default)]
    pub ie_key: Option<String>,
    #[serde(default)]
    webpage_url: Option<String>,
}

impl Entry {
    /// A playlist, not a video: it is opened, never downloaded as one.
    pub fn is_playlist(&self) -> bool {
        self.kind.as_deref() == Some("playlist")
            || self.ie_key.as_deref() == Some("YoutubeTab")
            || self.url.as_deref().is_some_and(|url| {
                reqwest::Url::parse(url).is_ok_and(|u| {
                    u.path() == "/playlist" && u.query_pairs().any(|(k, _)| k == "list")
                })
            })
    }
}

/// What [`VideoRunner::probe_sent`] found, and where.
#[derive(Debug, Clone, PartialEq)]
pub struct Sent {
    pub probed: Probed,
    /// The link that was read: the page's, or the video's own. The
    /// download goes from it.
    pub url: String,
    /// Whether the page's cookies were kept (see
    /// [`VideoRunner::probe_with_or_without_cookies`]).
    pub cookies_kept: bool,
}

/// What a page holds.
#[derive(Debug, Clone, PartialEq)]
pub enum Probed {
    Video(Video),
    Playlist { title: String, entries: Vec<Entry> },
}

/// Only the fields Zajil uses are read from the probe's JSON: the rest
/// (each format's `http_headers` and `cookies` among them) is never kept,
/// stored or printed (AGENTS.md, section 10.3).
#[derive(Deserialize)]
struct Raw {
    #[serde(rename = "_type", default)]
    kind: Option<String>,
    #[serde(default)]
    title: String,
    #[serde(default)]
    duration: Option<f64>,
    #[serde(default)]
    formats: Vec<Format>,
    #[serde(default)]
    entries: Vec<Entry>,
}

/// What a download reports while it runs.
#[derive(Debug, Clone, PartialEq)]
pub enum VideoEvent {
    /// Bytes of the whole download so far (a merge's picture and sound
    /// together), and of all of it when known.
    Progress {
        done: u64,
        total: Option<u64>,
        speed: Option<u64>,
    },
    /// A part saved before could not be continued: it starts over, for
    /// this reason (Zajil's own way, on YouTube).
    StartedOver(String),
    /// What the log may say of how it goes (never a link).
    Note(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_format_s_url_is_never_shown() {
        let format: Format = serde_json::from_str(
            r#"{"format_id": "1", "protocol": "m3u8_native", "url": "https://cdn/x.m3u8?sig=SECRET"}"#,
        )
        .unwrap();
        assert!(format.is_hls());
        assert!(!format!("{format:?}").contains("SECRET"));
    }

    /// YouTube offers a video's own sound beside AI dubs in the same
    /// formats (users' reports, 2026-10-31): its own is taken, even when
    /// a dub has the higher rate; without marks, the rate decides.
    #[test]
    fn the_best_choice_takes_the_video_s_own_sound() {
        let sound = |id: &str, tbr: f64, pref: Option<i64>, note: &str| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "ext": "m4a", "vcodec": "none", "acodec": "mp4a.40.2",
                "tbr": tbr, "language_preference": pref, "format_note": note,
            }))
            .unwrap()
        };
        let picture: Format = serde_json::from_value(serde_json::json!({
            "format_id": "137", "ext": "mp4", "height": 1080, "vcodec": "avc1.640028", "acodec": "none",
        }))
        .unwrap();
        let video = |sounds: Vec<Format>| Video {
            title: String::new(),
            duration: None,
            formats: std::iter::once(picture.clone()).chain(sounds).collect(),
        };
        let merge = |audio: &str| {
            Some(Choice::Merge {
                video: "137".into(),
                audio: audio.into(),
            })
        };
        let dubbed = sound("140-0", 130.0, Some(-1), "Arabic (dubbed-auto), medium");
        let own = sound(
            "140-1",
            129.0,
            Some(10),
            "English original (default), medium",
        );
        assert_eq!(
            video(vec![dubbed.clone(), own.clone()]).best_choice(),
            merge("140-1")
        );
        assert_eq!(
            video(vec![own.clone(), dubbed.clone()]).best_choice(),
            merge("140-1")
        );
        // Marked by its note only.
        let noted = sound("140-2", 100.0, None, "English original, medium");
        let plain = sound("140-3", 200.0, None, "Arabic, medium");
        assert_eq!(
            video(vec![plain.clone(), noted]).best_choice(),
            merge("140-2")
        );
        assert!(own.sound_rank() > dubbed.sound_rank());
        // Nothing marked: the higher rate.
        let low = sound("140", 100.0, None, "medium");
        assert_eq!(video(vec![low, plain]).best_choice(), merge("140-3"));
    }

    #[test]
    fn the_best_choice_prefers_higher_pictures_and_keeps_sound() {
        let f = |id: &str, h: Option<u32>, v: &str, a: &str| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "height": h, "vcodec": v, "acodec": a, "tbr": 100.0,
            }))
            .unwrap()
        };
        let video = |formats| Video {
            title: String::new(),
            duration: None,
            formats,
        };
        // A higher picture alone beats a lower one with sound: merged.
        assert_eq!(
            video(vec![
                f("18", Some(360), "avc1", "mp4a"),
                f("137", Some(1080), "avc1", "none"),
                f("140", None, "none", "mp4a")
            ])
            .best_choice(),
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        // Picture with sound at the top height: that one.
        assert_eq!(
            video(vec![
                f("22", Some(720), "avc1", "mp4a"),
                f("136", Some(720), "avc1", "none"),
                f("140", None, "none", "mp4a")
            ])
            .best_choice(),
            Some(Choice::Format("22".into()))
        );
        // Sound only.
        assert_eq!(
            video(vec![f("140", None, "none", "mp4a")]).best_choice(),
            Some(Choice::AudioOnly)
        );
        // A direct picture and sound before HLS with both, at the same
        // height (YouTube's itag 96 beside 137 and 140): Zajil's engine.
        let mut hls = f("96", Some(1080), "avc1.640028", "mp4a.40.2");
        hls.protocol = Some("m3u8_native".into());
        assert_eq!(
            video(vec![
                hls.clone(),
                f("18", Some(360), "avc1.42001E", "mp4a.40.2"),
                f("137", Some(1080), "avc1.640028", "none"),
                f("140", None, "none", "mp4a.40.2"),
            ])
            .best_choice(),
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        // HLS only: it still goes.
        assert_eq!(
            video(vec![hls]).best_choice(),
            Some(Choice::Format("96".into()))
        );
        // Unknown codecs (a direct file): the last.
        let mut plain = f("0", None, "x", "x");
        plain.vcodec = None;
        plain.acodec = None;
        assert_eq!(
            video(vec![plain]).best_choice(),
            Some(Choice::Format("0".into()))
        );
        assert_eq!(video(vec![]).best_choice(), None);
    }

    /// A YouTube page: VP9 and AV1 pictures above the H.264 one, Opus
    /// sound with a higher rate than AAC. The default is the MP4 that
    /// every player opens.
    #[test]
    fn the_best_choice_is_an_mp4_when_the_page_has_one() {
        let f = |id: &str, h: Option<u32>, ext: &str, v: &str, a: &str, tbr: f64| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "height": h, "ext": ext, "vcodec": v, "acodec": a, "tbr": tbr,
            }))
            .unwrap()
        };
        let page = Video {
            title: String::new(),
            duration: None,
            formats: vec![
                f("18", Some(360), "mp4", "avc1.42001E", "mp4a.40.2", 500.0),
                f("139", None, "m4a", "none", "mp4a.40.5", 49.0),
                f("140", None, "m4a", "none", "mp4a.40.2", 129.0),
                f("251", None, "webm", "none", "opus", 135.0),
                f("251-drc", None, "webm", "none", "opus", 160.0),
                f("137", Some(1080), "mp4", "avc1.640028", "none", 4000.0),
                f("399", Some(1080), "mp4", "av01.0.08M.08", "none", 2000.0),
                f("616", Some(1440), "mp4", "vp09.00.40.08", "none", 9000.0),
            ],
        };
        assert_eq!(
            page.best_choice(),
            Some(Choice::Merge {
                video: "137".into(),
                audio: "140".into()
            })
        );
        // An H.264 with its sound beside only a higher VP9 picture
        // (Instagram): the one every player opens.
        let instagram = Video {
            title: String::new(),
            duration: None,
            formats: vec![
                f("8", Some(1280), "mp4", "avc1.64001F", "mp4a.40.2", 1500.0),
                f(
                    "dash-1v",
                    Some(2560),
                    "mp4",
                    "vp09.00.51.08",
                    "none",
                    3000.0,
                ),
                f("dash-2a", None, "m4a", "none", "mp4a.40.5", 64.0),
            ],
        };
        assert_eq!(instagram.best_choice(), Some(Choice::Format("8".into())));
        // Facebook: its own MP4 files say no codecs, its DASH picture is
        // VP9: the MP4 file.
        let plain = |id: &str, h: u32| {
            let mut format = f(id, Some(h), "mp4", "x", "x", 0.0);
            format.vcodec = None;
            format.acodec = None;
            format
        };
        let facebook = Video {
            title: String::new(),
            duration: None,
            formats: vec![
                plain("sd", 360),
                plain("hd", 720),
                f("1v", Some(1280), "mp4", "vp09.00.40.08", "none", 2000.0),
                f("1a", None, "m4a", "none", "mp4a.40.5", 64.0),
            ],
        };
        assert_eq!(facebook.best_choice(), Some(Choice::Format("hd".into())));
        // No H.264 at all: the highest picture, with AAC still first.
        let mut other = page.clone();
        other
            .formats
            .retain(|f| !f.vcodec.as_deref().unwrap().starts_with("avc1"));
        assert_eq!(
            other.best_choice(),
            Some(Choice::Merge {
                video: "616".into(),
                audio: "140".into()
            })
        );
    }

    #[test]
    fn drm_formats_are_recognised() {
        let f = |v: serde_json::Value| Format {
            id: "1".into(),
            ext: "mp4".into(),
            width: None,
            height: None,
            fps: None,
            vcodec: None,
            acodec: None,
            filesize: None,
            filesize_approx: None,
            tbr: None,
            has_drm: Some(v),
            protocol: None,
            url: None,
            agent: None,
            language: None,
            language_preference: None,
            format_note: None,
        };
        assert!(f(serde_json::json!(true)).is_protected());
        assert!(f(serde_json::json!("maybe")).is_protected());
        assert!(!f(serde_json::json!(false)).is_protected());
    }
}
