//! What the page is told of a video: its formats, the choice, a
//! playlist's entries and the errors.

use serde::{Deserialize, Serialize};
use ts_rs::TS;

use zajil_core::video::{Choice, Format, ToolProblem, VideoError};

/// A format, for «Video quality».
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct FormatView {
    pub id: String,
    pub ext: String,
    pub width: Option<u32>,
    pub height: Option<u32>,
    pub fps: Option<f64>,
    /// Its size, or the site's guess of it.
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    /// `both`, `video` (picture only) or `audio`.
    pub kind: String,
    pub hls: bool,
    /// The picture's codec as people know it (`H.264`, `VP9`, `AV1`,
    /// `HEVC`), if the site says it: QuickTime plays H.264 and HEVC only
    /// (owner's report, 2026-10-30).
    pub codec: Option<String>,
    /// A sound that is the video's own (not a dubbed one), when the page
    /// offers several (users' reports, 2026-10-31).
    pub original: bool,
    /// A sound's language (`en`, `ar-EG`…), if the site says it: «Video
    /// quality» lets the user take a dubbed one (owner's request,
    /// 2026-10-31).
    pub language: Option<String>,
}

/// A picture codec's name as people know it, from yt-dlp's `vcodec`.
fn codec_name(vcodec: &str) -> Option<&'static str> {
    let v = vcodec.to_ascii_lowercase();
    [
        (["avc1", "h264"], "H.264"),
        (["vp09", "vp9"], "VP9"),
        (["vp08", "vp8"], "VP8"),
        (["av01", "av1"], "AV1"),
        (["hvc1", "hev1"], "HEVC"),
    ]
    .into_iter()
    .find(|(starts, _)| starts.iter().any(|s| v.starts_with(s)))
    .map(|(_, name)| name)
}

/// What to download, as the page says it.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum ChoiceView {
    Format {
        id: String,
    },
    Merge {
        video: String,
        audio: String,
    },
    AudioOnly,
    /// The engine's best, chosen when it starts and its page is asked (a
    /// playlist's items); then kept as that choice.
    Best,
}

impl From<Choice> for ChoiceView {
    fn from(c: Choice) -> Self {
        match c {
            Choice::Format(id) => ChoiceView::Format { id },
            Choice::Merge { video, audio } => ChoiceView::Merge { video, audio },
            Choice::AudioOnly => ChoiceView::AudioOnly,
        }
    }
}

impl ChoiceView {
    /// The engine's choice, with [`ChoiceView::Best`] chosen among
    /// `formats`.
    pub(super) fn resolved(&self, formats: &[Format]) -> Result<Choice, VideoErrorView> {
        Ok(match self.clone() {
            ChoiceView::Format { id } => Choice::Format(id),
            ChoiceView::Merge { video, audio } => Choice::Merge { video, audio },
            ChoiceView::AudioOnly => Choice::AudioOnly,
            ChoiceView::Best => zajil_core::video::Video {
                title: String::new(),
                duration: None,
                formats: formats.to_vec(),
            }
            .best_choice()
            .ok_or_else(|| VideoErrorView {
                kind: "no_formats".into(),
                message: "the page offers no format to download".into(),
            })?,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[ts(export)]
pub struct EntryView {
    pub url: Option<String>,
    pub title: Option<String>,
    /// A playlist inside the playlist: opened, not downloaded.
    pub playlist: bool,
}

/// What a page holds.
#[derive(Debug, Clone, PartialEq, Serialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum VideoInfoView {
    Video {
        title: String,
        duration: Option<f64>,
        formats: Vec<FormatView>,
        best: Option<ChoiceView>,
    },
    Playlist {
        title: String,
        entries: Vec<EntryView>,
    },
}

/// Why it cannot: its kind, for the page's words, and the engine's text
/// (safe to show: no cookie, no token).
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct VideoErrorView {
    /// `drm`, `tools_missing`, `refused`, `tool`, `output`, `stopped`,
    /// `io`, or `not_ready` when yt-dlp did not start after its download;
    /// for a failure of yt-dlp's whose words say what it comes to,
    /// `no_formats`, `sign_in`, `private`, `unsupported` or `challenge`;
    /// `playlist` for a playlist's item that is a playlist itself.
    pub kind: String,
    pub message: String,
}

impl From<VideoError> for VideoErrorView {
    fn from(e: VideoError) -> Self {
        let kind = match &e {
            VideoError::Drm => "drm",
            VideoError::Tools(_) => "tools_missing",
            VideoError::Refused(_) => "refused",
            VideoError::Tool(_) => match e.problem() {
                Some(ToolProblem::NoFormats) => "no_formats",
                Some(ToolProblem::SignIn) => "sign_in",
                Some(ToolProblem::Private) => "private",
                Some(ToolProblem::Unsupported) => "unsupported",
                Some(ToolProblem::Challenge) => "challenge",
                None => "tool",
            },
            VideoError::Output(_) => "output",
            VideoError::Stopped => "stopped",
            VideoError::Io(_) => "io",
            VideoError::NotReady(_) => "not_ready",
        };
        VideoErrorView {
            kind: kind.into(),
            message: e.to_string(),
        }
    }
}

impl VideoErrorView {
    /// What the list says of it, in `lang`: a sentence, and yt-dlp's own
    /// words after it when no sentence says what it comes to.
    pub(super) fn text(&self, lang: zajil_core::settings::Language) -> String {
        let lang = match lang {
            zajil_core::settings::Language::Ar => zajil_i18n::Lang::Ar,
            zajil_core::settings::Language::En => zajil_i18n::Lang::En,
        };
        let localizer = zajil_i18n::Localizer::new(lang);
        let key = match self.kind.as_str() {
            "drm" => "video-drm",
            "tools_missing" => "video-tools-missing",
            "no_formats" => "video-error-no_formats",
            "sign_in" => "video-error-sign_in",
            "private" => "video-error-private",
            "unsupported" => "video-error-unsupported",
            "challenge" => "video-error-challenge",
            "playlist" => "video-error-playlist",
            "not_ready" => "video-tools-not-ready",
            _ => {
                return localizer
                    .text(
                        "video-error-other-details",
                        &[("details", zajil_i18n::Arg::Text(&self.message))],
                    )
                    .unwrap_or_else(|_| self.message.clone());
            }
        };
        localizer
            .text(key, &[])
            .unwrap_or_else(|_| self.message.clone())
    }
}

pub fn format_view(f: &Format) -> FormatView {
    // Codecs the site does not say (X's direct MP4s, a plain file): a
    // picture with its sound when it has a picture's size or a video's
    // extension, as yt-dlp's own «best» takes it (owner's report,
    // 2026-10-29: X offered sound only).
    let unknown = f.vcodec.is_none() && f.acodec.is_none();
    let a_picture = f.height.is_some()
        || f.width.is_some()
        || matches!(
            f.ext.as_str(),
            "mp4" | "webm" | "mov" | "mkv" | "flv" | "3gp"
        );
    let kind = match (f.has_video(), f.has_audio()) {
        (true, true) => "both",
        (true, false) => "video",
        _ if unknown && a_picture => "both",
        _ => "audio",
    };
    FormatView {
        id: f.id.clone(),
        ext: f.ext.clone(),
        width: f.width,
        height: f.height,
        fps: f.fps,
        size: f.filesize.or(f.filesize_approx),
        kind: kind.into(),
        hls: f.is_hls(),
        codec: f.vcodec.as_deref().and_then(codec_name).map(Into::into),
        original: false,
        language: f.language.clone().filter(|l| {
            !l.is_empty()
                && l.len() <= 35
                && l.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-')
        }),
    }
}

/// The views of `formats`, the video's own sounds marked: those yt-dlp
/// ranks highest.
pub fn format_views(formats: &[Format]) -> Vec<FormatView> {
    let sound = |f: &&Format| f.has_audio() && !f.has_video();
    let top = formats.iter().filter(sound).map(Format::sound_rank).max();
    formats
        .iter()
        .map(|f| FormatView {
            original: sound(&f) && Some(f.sound_rank()) == top,
            ..format_view(f)
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    /// The page's names for what can go wrong, one per kind.
    #[test]
    fn every_error_has_its_kind() {
        let kinds: Vec<String> = [
            VideoError::Drm,
            VideoError::Refused("no".into()),
            VideoError::Tool("x".into()),
            VideoError::Output("x".into()),
            VideoError::Stopped,
            VideoError::Io(std::io::Error::other("x")),
        ]
        .into_iter()
        .map(|e| VideoErrorView::from(e).kind)
        .collect();
        assert_eq!(kinds, ["drm", "refused", "tool", "output", "stopped", "io"]);
    }

    /// A format as the page sees it: its kind from its codecs, its size
    /// or the site's guess, and whether it is HLS.
    #[test]
    fn formats_say_their_kind_and_size() {
        let f = |json: &str| format_view(&serde_json::from_str::<Format>(json).unwrap());
        let both = f(
            r#"{"format_id":"22","ext":"mp4","width":1280,"height":720,"vcodec":"avc1","acodec":"mp4a","filesize":1000}"#,
        );
        assert_eq!(
            (both.kind.as_str(), both.size, both.height),
            ("both", Some(1000), Some(720))
        );
        let picture = f(
            r#"{"format_id":"137","ext":"mp4","vcodec":"avc1","acodec":"none","filesize_approx":5000}"#,
        );
        assert_eq!((picture.kind.as_str(), picture.size), ("video", Some(5000)));
        let sound = f(r#"{"format_id":"140","ext":"m4a","vcodec":"none","acodec":"mp4a"}"#);
        assert_eq!((sound.kind.as_str(), sound.size), ("audio", None));
        let hls = f(
            r#"{"format_id":"hls-720","ext":"mp4","vcodec":"avc1","acodec":"mp4a","protocol":"m3u8_native"}"#,
        );
        assert!(hls.hls);
        assert_eq!(both.codec.as_deref(), Some("H.264"));
        let vp9 =
            f(r#"{"format_id":"dash-1v","ext":"mp4","vcodec":"vp09.00.51.08","acodec":"none"}"#);
        assert_eq!(vp9.codec.as_deref(), Some("VP9"));
        assert_eq!(sound.codec, None);
        // X's direct MP4: no codecs said, a picture's size.
        let x = f(r#"{"format_id":"http-2176","ext":"mp4","width":1280,"height":720}"#);
        assert_eq!((x.kind.as_str(), x.height), ("both", Some(720)));
        let plain = f(r#"{"format_id":"0","ext":"mp4"}"#);
        assert_eq!(plain.kind, "both");
        // Nothing said, and no picture's sign: sound.
        let mp3 = f(r#"{"format_id":"0","ext":"mp3"}"#);
        assert_eq!(mp3.kind, "audio");
    }

    /// What the page sends comes back as the engine's choice, and back.
    #[test]
    fn choices_go_both_ways() {
        for choice in [
            Choice::Format("22".into()),
            Choice::Merge {
                video: "137".into(),
                audio: "140".into(),
            },
            Choice::AudioOnly,
        ] {
            assert_eq!(ChoiceView::from(choice.clone()).resolved(&[]), Ok(choice));
        }
        let json: ChoiceView =
            serde_json::from_str(r#"{"kind":"merge","video":"137","audio":"140"}"#).unwrap();
        assert_eq!(
            json,
            ChoiceView::Merge {
                video: "137".into(),
                audio: "140".into()
            }
        );
    }

    /// A failed video says in the list what it comes to, in the user's
    /// language; yt-dlp's words follow only when no sentence names it.
    #[test]
    fn a_failure_is_told_in_the_user_s_language() {
        use zajil_core::settings::Language;
        let known = VideoErrorView::from(VideoError::Tool(
            "[youtube] abc: Sign in to confirm your age".into(),
        ));
        assert_eq!(known.kind, "sign_in");
        let text = known.text(Language::Ar);
        assert!(text.contains("تسجيل الدخول"), "{text}");
        assert!(!text.contains("Sign in"), "{text}");
        let other = VideoErrorView::from(VideoError::Tool("HTTP Error 500".into()));
        assert_eq!(other.kind, "tool");
        let text = other.text(Language::Ar);
        assert!(
            text.contains("تعذّر تحميل هذا الفيديو") && text.contains("HTTP Error 500"),
            "{text}"
        );
        assert!(
            VideoErrorView::from(VideoError::Drm)
                .text(Language::En)
                .contains("protected")
        );
    }
}
