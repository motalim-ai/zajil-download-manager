//! The format chosen in the browser's list or the window, matched to the
//! formats yt-dlp finds.

use super::views::ChoiceView;
use zajil_core::video::Format;

/// Whether two choices are the same formats, by YouTube's itags (yt-dlp
/// names a format `140-0` where the list says `140`).
pub(super) fn same_itags(a: &ChoiceView, b: &ChoiceView) -> bool {
    let itag = |id: &str| id.split('-').next().unwrap_or_default().to_owned();
    match (a, b) {
        (ChoiceView::Format { id: x }, ChoiceView::Format { id: y }) => itag(x) == itag(y),
        (
            ChoiceView::Merge {
                video: v1,
                audio: a1,
            },
            ChoiceView::Merge {
                video: v2,
                audio: a2,
            },
        ) => itag(v1) == itag(v2) && itag(a1) == itag(a2),
        _ => false,
    }
}

/// `choice` if this read of the page still offers its formats, else the
/// engine's best: YouTube numbers an HLS format anew at each read (`96-2`,
/// then `96-1`), so a stopped download chosen so could never go on (owner's
/// report, 2026-10-29: «the chosen format is not one the probe found»).
/// Nothing read (no formats): as it was.
pub(super) fn still_offered(choice: &ChoiceView, formats: &[Format]) -> ChoiceView {
    let there = |id: &String| formats.iter().any(|f| &f.id == id);
    let offered = match choice {
        ChoiceView::Format { id } => there(id),
        ChoiceView::Merge { video, audio } => there(video) && there(audio),
        ChoiceView::AudioOnly | ChoiceView::Best => true,
    };
    if offered || formats.is_empty() {
        choice.clone()
    } else {
        ChoiceView::Best
    }
}

/// The format of `formats` that YouTube's itag `wanted` names: yt-dlp
/// names a format by its itag, with a suffix for another language's sound
/// or a "stable volume" one (`140-drc`). Of those, the video's own sound
/// before a dubbed one (users' reports, 2026-10-31), then the plain one.
fn by_itag(formats: &[Format], wanted: &str) -> Option<String> {
    // A dub chosen in the browser's list: its itag and language (`140-ar`,
    // owner's request, 2026-10-31); that language's sound, else as usual.
    if !formats.iter().any(|f| f.id == wanted)
        && let Some((itag, language)) = wanted.split_once('-')
        && language.starts_with(|c: char| c.is_ascii_alphabetic())
        && language != "drc"
    {
        let lang = |f: &Format| {
            f.language.as_deref().is_some_and(|l| {
                l.eq_ignore_ascii_case(language)
                    || l.split('-')
                        .next()
                        .is_some_and(|b| b.eq_ignore_ascii_case(language))
            })
        };
        return formats
            .iter()
            .find(|f| {
                lang(f)
                    && f.id
                        .split_once('-')
                        .map_or(f.id == itag, |(i, rest)| i == itag && !rest.contains("drc"))
            })
            .map(|f| f.id.clone())
            .or_else(|| by_itag(formats, itag));
    }
    let mut best: Option<&Format> = None;
    for f in formats.iter().filter(|f| {
        f.id == wanted
            || f.id
                .split_once('-')
                .is_some_and(|(itag, rest)| itag == wanted && !rest.contains("drc"))
    }) {
        // Ties keep the first: the plain one, as yt-dlp lists it.
        let wins = match best {
            None => true,
            Some(b) => {
                f.sound_rank() > b.sound_rank()
                    || (f.sound_rank() == b.sound_rank() && f.id == wanted && b.id != wanted)
            }
        };
        if wins {
            best = Some(f);
        }
    }
    best.map(|f| f.id.clone())
}

/// A choice made in the browser's list, in the names of yt-dlp's formats;
/// the engine's best when the probe did not find what was chosen.
pub(super) fn from_list(choice: &ChoiceView, formats: &[Format]) -> ChoiceView {
    let found = match choice {
        ChoiceView::Format { id } => by_itag(formats, id).map(|id| ChoiceView::Format { id }),
        ChoiceView::Merge { video, audio } => by_itag(formats, video)
            .zip(by_itag(formats, audio))
            .map(|(video, audio)| ChoiceView::Merge { video, audio }),
        other => Some(other.clone()),
    };
    found.unwrap_or(ChoiceView::Best)
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A choice from the browser's list names YouTube's itags; yt-dlp's
    /// formats may add a suffix, and a "stable volume" sound is not taken.
    #[test]
    fn a_choice_from_the_list_finds_yt_dlps_format() {
        let format = |id: &str| -> Format {
            serde_json::from_value(serde_json::json!({ "format_id": id })).unwrap()
        };
        let formats = [
            format("299"),
            format("140-drc"),
            format("140-1"),
            format("251"),
        ];
        assert_eq!(
            from_list(
                &ChoiceView::Merge {
                    video: "299".into(),
                    audio: "140".into()
                },
                &formats
            ),
            ChoiceView::Merge {
                video: "299".into(),
                audio: "140-1".into()
            }
        );
        assert_eq!(
            from_list(&ChoiceView::Format { id: "251".into() }, &formats),
            ChoiceView::Format { id: "251".into() }
        );
        // The video's own sound before a dub of the same itag (users'
        // reports, 2026-10-31), wherever yt-dlp lists it.
        let sound = |id: &str, pref: i64| -> Format {
            serde_json::from_value(
                serde_json::json!({ "format_id": id, "language_preference": pref }),
            )
            .unwrap()
        };
        let dubbed = [sound("140-0", -1), sound("140-1", 10), sound("140-2", -1)];
        assert_eq!(by_itag(&dubbed, "140").as_deref(), Some("140-1"));
        let plain_first = [format("140"), format("140-1")];
        assert_eq!(by_itag(&plain_first, "140").as_deref(), Some("140"));
        // A dub chosen in the list: its language's sound (owner's request,
        // 2026-10-31); a language the page lacks: the video's own.
        let spoken = |id: &str, pref: i64, language: &str| -> Format {
            serde_json::from_value(serde_json::json!({
                "format_id": id, "language_preference": pref, "language": language,
            }))
            .unwrap()
        };
        let tracks = [
            spoken("140-0", -1, "ar"),
            spoken("140-drc", -1, "ar"),
            spoken("140-1", 10, "en-US"),
            spoken("251-0", -1, "ar"),
        ];
        assert_eq!(by_itag(&tracks, "140-ar").as_deref(), Some("140-0"));
        assert_eq!(by_itag(&tracks, "140-en").as_deref(), Some("140-1"));
        assert_eq!(by_itag(&tracks, "140-fr").as_deref(), Some("140-1"));
        assert_eq!(
            from_list(
                &ChoiceView::Merge {
                    video: "299".into(),
                    audio: "140-ar".into()
                },
                &[
                    format("299"),
                    spoken("140-0", -1, "ar"),
                    spoken("140-1", 10, "en")
                ]
            ),
            ChoiceView::Merge {
                video: "299".into(),
                audio: "140-0".into()
            }
        );
        assert_eq!(
            from_list(&ChoiceView::Format { id: "18".into() }, &formats),
            ChoiceView::Best
        );
    }

    /// A stopped download whose format this read no longer offers (an HLS
    /// one, numbered anew) goes at the engine's best; one still offered,
    /// or with nothing read, as chosen.
    #[test]
    fn a_format_gone_since_goes_at_the_best() {
        let f = |id: &str| -> Format {
            serde_json::from_value(
                serde_json::json!({ "format_id": id, "vcodec": "avc1", "acodec": "mp4a" }),
            )
            .unwrap()
        };
        let read = [f("96-1"), f("137"), f("140")];
        let format = |id: &str| ChoiceView::Format { id: id.into() };
        assert_eq!(still_offered(&format("96-2"), &read), ChoiceView::Best);
        assert_eq!(still_offered(&format("96-1"), &read), format("96-1"));
        let merge = ChoiceView::Merge {
            video: "137".into(),
            audio: "140".into(),
        };
        assert_eq!(still_offered(&merge, &read), merge);
        assert_eq!(still_offered(&format("96-2"), &[]), format("96-2"));
    }
}
