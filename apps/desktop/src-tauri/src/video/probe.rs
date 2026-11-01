//! A page's video asked about: «Video quality»'s check, and the pages
//! read ahead while the extension's button shows over a video.

use std::collections::HashMap;
use std::time::{Duration, Instant};

use tauri::{Manager, State};
use zajil_core::Stop;

use super::views::{
    ChoiceView, EntryView, VideoErrorView, VideoInfoView, format_view, format_views,
};
use super::{VideoState, runner};
use zajil_core::video::{Probed, VideoError, VideoRunner};

/// How long a video read ahead is kept (owner, 2026-10-28): in memory
/// only, never written anywhere.
const AHEAD_FOR: Duration = Duration::from_secs(10 * 60);
/// At most this many are read ahead at once; another one meanwhile is not
/// (yt-dlp solving YouTube's JavaScript takes seconds of a core each).
const AHEAD_RUNNING: usize = 2;
/// At most this many are kept; the oldest goes first.
const AHEAD_KEPT: usize = 20;

/// A read ahead: none while it runs, then what it found (none if it
/// failed: the click asks again).
type AheadRead = tokio::sync::watch::Receiver<Option<Option<zajil_core::video::Sent>>>;
type AheadDone = tokio::sync::watch::Sender<Option<Option<zajil_core::video::Sent>>>;

/// The videos read ahead (owner, 2026-10-28, as IDM): when the extension's
/// button shows over a video, its page is read in the background once, so
/// its qualities are there when the button is pressed. Never from a
/// private window (the extension sends none, and the host refuses them).
#[derive(Default)]
pub(super) struct Aheads(HashMap<String, Ahead>);

struct Ahead {
    at: Instant,
    cookies: bool,
    read: AheadRead,
}

impl Aheads {
    fn forget_old(&mut self, now: Instant) {
        self.0
            .retain(|_, a| now.saturating_duration_since(a.at) < AHEAD_FOR);
    }

    /// Notes a read of `page` and returns where to put what it found;
    /// none when it was read in the last ten minutes, or is being read,
    /// or [`AHEAD_RUNNING`] are running.
    fn begin(&mut self, page: &str, cookies: bool, now: Instant) -> Option<AheadDone> {
        self.forget_old(now);
        let running = self.0.values().filter(|a| a.read.borrow().is_none());
        if self.0.contains_key(page) || running.count() >= AHEAD_RUNNING {
            return None;
        }
        if self.0.len() >= AHEAD_KEPT {
            let oldest = self.0.iter().min_by_key(|(_, a)| a.at);
            if let Some(oldest) = oldest.map(|(page, _)| page.clone()) {
                self.0.remove(&oldest);
            }
        }
        let (done, read) = tokio::sync::watch::channel(None);
        let ahead = Ahead {
            at: now,
            cookies,
            read,
        };
        self.0.insert(page.to_owned(), ahead);
        Some(done)
    }

    /// The read of `page` the button's press can take: one of the last
    /// ten minutes, with cookies if the press has some and without if not
    /// (signed in or not, yt-dlp finds other formats). What it found must
    /// still fit the press ([`ahead_fits`]).
    pub(super) fn find(&mut self, page: &str, cookies: bool, now: Instant) -> Option<AheadRead> {
        self.forget_old(now);
        self.0
            .get(page)
            .filter(|a| a.cookies == cookies)
            .map(|a| a.read.clone())
    }
}

/// Whether what a read ahead found fits a press on `page` with the video's
/// own link `media`: the page itself was read, or the video's link the
/// press has too (the one seen at the press may be another, YouTube's
/// above all: only the page counts then).
pub(super) fn ahead_fits(sent: &zajil_core::video::Sent, page: &str, media: Option<&str>) -> bool {
    sent.url == page || media == Some(sent.url.as_str())
}

#[tauri::command]
pub async fn video_probe(
    app: State<'_, crate::App>,
    state: State<'_, VideoState>,
    url: String,
) -> Result<VideoInfoView, VideoErrorView> {
    let stop = Stop::new();
    if let Some(old) = state.probing.lock().unwrap().replace(stop.clone()) {
        old.request();
    }
    let cookies = state.cookies.lock().unwrap().get(&url).cloned();
    let media = state.media.lock().unwrap().get(&url).cloned();
    // Read ahead while the pointer was on it: taken, or waited for.
    let ahead = state
        .ahead
        .lock()
        .unwrap()
        .find(&url, cookies.is_some(), Instant::now());
    let read = match ahead {
        Some(mut ahead) => tokio::select! {
            read = ahead.wait_for(Option::is_some) => read
                .ok()
                .and_then(|r| r.clone().flatten())
                .filter(|sent| ahead_fits(sent, &url, media.as_deref())),
            () = stop.requested() => return Err(VideoError::Stopped.into()),
        },
        None => None,
    };
    let sent = match read {
        Some(sent) => sent,
        None => {
            runner(&app)
                .await
                .probe_sent(&url, media.as_deref(), cookies.as_deref(), &stop)
                .await?
        }
    };
    state.probing.lock().unwrap().take();
    if !sent.cookies_kept {
        state.cookies.lock().unwrap().remove(&url);
    }
    // Read by the video's own link: the download goes from that one.
    if sent.url != url {
        state
            .read_as
            .lock()
            .unwrap()
            .insert(url.clone(), sent.url.clone());
    }
    // Never the links (they may carry a token): which was read, and what.
    if let Probed::Video(video) = &sent.probed {
        let pictures = video
            .formats
            .iter()
            .filter(|f| format_view(f).kind != "audio")
            .count();
        eprintln!(
            "zajil: read from {}: {} formats, {pictures} with a picture",
            if sent.url == url {
                "the page"
            } else {
                "the page's playlist"
            },
            video.formats.len(),
        );
        // Each format's height and codecs, never its link (owner's
        // reports, 2026-10-30: which picture a site offers).
        let each: Vec<String> = video
            .formats
            .iter()
            .map(|f| {
                // A sound's language and rank: the video's own or dubbed.
                let sound = if f.has_audio() && !f.has_video() {
                    format!(
                        " [{} {}]",
                        f.language.as_deref().unwrap_or("?"),
                        f.sound_rank()
                    )
                } else {
                    String::new()
                };
                format!(
                    "{}p {} {}/{}{sound}",
                    f.height.unwrap_or(0),
                    f.ext,
                    f.vcodec.as_deref().unwrap_or("?"),
                    f.acodec.as_deref().unwrap_or("?")
                )
            })
            .collect();
        eprintln!("zajil: the formats: {}", each.join(", "));
    }
    let read = sent.url;
    Ok(match sent.probed {
        Probed::Video(video) => {
            let best = video.best_choice().map(ChoiceView::from);
            let formats = format_views(&video.formats);
            state
                .formats
                .lock()
                .unwrap()
                .insert(read, video.formats.clone());
            VideoInfoView::Video {
                title: video.title,
                duration: video.duration,
                formats,
                best,
            }
        }
        Probed::Playlist { title, entries } => VideoInfoView::Playlist {
            title,
            entries: entries
                .into_iter()
                .map(|e| EntryView {
                    playlist: e.is_playlist(),
                    url: e.url,
                    title: e.title,
                })
                .collect(),
        },
    })
}

/// The extension's button showed over a video: its page is read now, in
/// the background, unless it was read in the last ten minutes ([`Aheads`]).
/// Nothing is shown, and nothing of it is kept but in memory.
pub fn read_ahead<R: tauri::Runtime>(
    handle: &tauri::AppHandle<R>,
    page: String,
    media: Option<String>,
    cookies: Option<String>,
) {
    let begun = handle.state::<VideoState>().ahead.lock().unwrap().begin(
        &page,
        cookies.is_some(),
        Instant::now(),
    );
    let Some(done) = begun else {
        return;
    };
    let handle = handle.clone();
    tauri::async_runtime::spawn(async move {
        let runner = runner(&handle.state::<crate::App>()).await;
        let read = runner
            .probe_sent(&page, media.as_deref(), cookies.as_deref(), &Stop::new())
            .await
            .ok();
        let _ = done.send(Some(read));
    });
}

/// Asks `url`'s page, with its cookies from the browser if any, else
/// without them ([`VideoRunner::probe_with_or_without_cookies`]). Cookies
/// that left no format are forgotten, so the download goes the same way.
pub(super) async fn probe(
    state: &VideoState,
    runner: &VideoRunner,
    url: &str,
    stop: &Stop,
) -> Result<Probed, VideoError> {
    let cookies = state.cookies.lock().unwrap().get(url).cloned();
    let (probed, kept) = runner
        .probe_with_or_without_cookies(url, cookies.as_deref(), stop)
        .await?;
    if !kept {
        state.cookies.lock().unwrap().remove(url);
    }
    Ok(probed)
}

#[tauri::command]
pub fn video_probe_cancel(state: State<'_, VideoState>) {
    if let Some(stop) = state.probing.lock().unwrap().take() {
        stop.request();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sent(page: &str) -> zajil_core::video::Sent {
        zajil_core::video::Sent {
            probed: Probed::Playlist {
                title: String::new(),
                entries: Vec::new(),
            },
            url: page.into(),
            cookies_kept: false,
        }
    }

    /// A video is read ahead once in ten minutes, two at a time at most,
    /// and twenty are kept at most, the oldest going first (owner,
    /// 2026-10-28).
    #[test]
    fn a_video_is_read_ahead_once_in_ten_minutes_and_two_at_a_time() {
        let mut aheads = Aheads::default();
        let now = Instant::now();
        let a = aheads.begin("https://a.example/", true, now).unwrap();
        assert!(aheads.begin("https://a.example/", true, now).is_none());
        let b = aheads.begin("https://b.example/", false, now).unwrap();
        // Two run: a third waits for a later pass of the pointer.
        assert!(aheads.begin("https://c.example/", false, now).is_none());
        a.send(Some(None)).unwrap();
        assert!(aheads.begin("https://c.example/", false, now).is_some());
        // Read, even if it failed: not again before ten minutes.
        assert!(aheads.begin("https://a.example/", true, now).is_none());
        b.send(Some(Some(sent("https://b.example/")))).unwrap();
        let later = now + AHEAD_FOR;
        assert!(aheads.begin("https://a.example/", true, later).is_some());
        assert!(!aheads.0.contains_key("https://b.example/"));

        let mut aheads = Aheads::default();
        for i in 0..=AHEAD_KEPT {
            let at = now + Duration::from_secs(i as u64);
            let done = aheads.begin(&format!("https://{i}.example/"), false, at);
            done.unwrap().send(Some(None)).unwrap();
        }
        assert_eq!(aheads.0.len(), AHEAD_KEPT);
        assert!(!aheads.0.contains_key("https://0.example/"));
    }

    /// The press takes a read of its page, made with cookies if it has
    /// some and without if not, from the last ten minutes; it waits for one
    /// still running.
    #[test]
    fn the_press_takes_the_read_of_its_own_page_or_waits_for_it() {
        let mut aheads = Aheads::default();
        let now = Instant::now();
        let page = "https://www.youtube.com/watch?v=x";
        let done = aheads.begin(page, true, now).unwrap();
        assert!(aheads.find(page, false, now).is_none());
        assert!(
            aheads
                .find("https://www.youtube.com/watch?v=y", true, now)
                .is_none()
        );
        let mut read = aheads.find(page, true, now).unwrap();
        let waited = tauri::async_runtime::block_on(async move {
            tauri::async_runtime::spawn(async move {
                tokio::time::sleep(Duration::from_millis(50)).await;
                done.send(Some(Some(sent(page)))).unwrap();
            });
            read.wait_for(Option::is_some).await.unwrap().clone()
        });
        assert_eq!(waited, Some(Some(sent(page))));
        assert!(aheads.find(page, true, now + AHEAD_FOR).is_none());
    }

    /// A read of the page fits a press whatever video link it has (on
    /// YouTube it changes between the pointer's pass and the press); a
    /// read of the video's own link fits only a press with that link.
    #[test]
    fn a_read_of_the_page_fits_any_press_on_it_and_one_of_the_link_only_its_own() {
        let page = "https://www.youtube.com/watch?v=x";
        assert!(ahead_fits(&sent(page), page, None));
        assert!(ahead_fits(
            &sent(page),
            page,
            Some("https://cdn.example/other.m3u8")
        ));
        let media = "https://cdn.example/v.m3u8";
        assert!(ahead_fits(&sent(media), page, Some(media)));
        assert!(!ahead_fits(
            &sent(media),
            page,
            Some("https://cdn.example/w.m3u8")
        ));
        assert!(!ahead_fits(&sent(media), page, None));
    }
}
