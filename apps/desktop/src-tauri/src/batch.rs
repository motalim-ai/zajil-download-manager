//! «Batch download» (M7, as IDM's «Add batch download»): a link with a
//! pattern (`https://site.com/img*.jpg` with «from» and «to», or ranges and
//! lists such as `[001-100]` and `{a,b}`) made into its links with
//! `zajil batch`'s own rules (`zajil_grab::pattern`), shown before anything
//! is added, then added to a queue at once, started or not, as «Download
//! file info» adds one. The links are made again here when they are added:
//! the page's list is only shown, never trusted.

use std::sync::Mutex;

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use ts_rs::TS;
use zajil_core::queue::{QueueStore, Status, StoreError};
use zajil_grab::pattern::{self, PatternError, Star};

pub const LABEL: &str = "batch";
pub const SIZE: (f64, f64) = (680.0, 600.0);

/// The window's capability: events, its title, and the folder picker.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/batch.json");

/// The most links one batch makes, as `zajil batch` by default.
pub const MAX_URLS: u64 = pattern::DEFAULT_MAX_URLS;

/// The most links the preview lists (the count says how many in all).
pub const SHOWN: usize = 200;

/// The link the window starts with (from the clipboard or the drop box).
#[derive(Default)]
pub struct Pending(Mutex<Option<String>>);

/// A pattern, and the range of its `*`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BatchRequest {
    pub pattern: String,
    /// Where the `*` counter starts and ends (numbers or letters); `null`
    /// without a `*`.
    pub from: Option<String>,
    pub to: Option<String>,
    /// Digits of each number, with zeros on the left (3 gives 001).
    pub width: Option<u8>,
}

/// The links a pattern makes: how many, and the first ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct BatchPreview {
    #[ts(type = "number")]
    pub count: u64,
    pub shown: Vec<String>,
    #[ts(type = "number")]
    pub max: u64,
}

/// Why a pattern was refused, for the page to say in the user's language:
/// its kind, where (a character of the pattern, from 1), and the numbers
/// or text it is about.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct BatchProblem {
    /// `not_http`, `too_long`, `hidden`, `no_host`, `in_host`,
    /// `in_fragment`, `backslash`, `unclosed`, `unopened`, `nested`,
    /// `empty`, `bad_range`, `mixed`, `descending`, `zero_step`, `width`,
    /// `too_large`, `bad_counter`, `star_needs_range`, `second_star`,
    /// `no_star`, `no_pattern`, `too_many`, `bad_url`, `video_page`.
    pub kind: String,
    pub at: Option<usize>,
    pub text: Option<String>,
    #[ts(type = "number | null")]
    pub count: Option<u64>,
    #[ts(type = "number | null")]
    pub max: Option<u64>,
}

impl From<PatternError> for BatchProblem {
    fn from(e: PatternError) -> Self {
        use PatternError::*;
        let (kind, at, text, count, max) = match e {
            NotHttp => ("not_http", None, None, None, None),
            TooLong => ("too_long", None, None, None, None),
            ControlCharacter { at } => ("hidden", Some(at), None, None, None),
            NoHost => ("no_host", None, None, None, None),
            InFragment { at } => ("in_fragment", Some(at), None, None, None),
            Backslash { at } => ("backslash", Some(at), None, None, None),
            InHost { at } => ("in_host", Some(at), None, None, None),
            HostChanged { .. } => ("in_host", None, None, None, None),
            Unclosed { at, open } => ("unclosed", Some(at), Some(open.to_string()), None, None),
            Unopened { at, close } => ("unopened", Some(at), Some(close.to_string()), None, None),
            Nested { at } => ("nested", Some(at), None, None, None),
            Empty { at } => ("empty", Some(at), None, None, None),
            BadRange { at, text } => ("bad_range", Some(at), Some(text), None, None),
            Mixed { at, text } => ("mixed", Some(at), Some(text), None, None),
            Descending { at, text } => ("descending", Some(at), Some(text), None, None),
            ZeroStep { at } => ("zero_step", Some(at), None, None, None),
            PaddedEnd { at, .. } | WidthMismatch { at, .. } | WidthTooLarge { at, .. } => {
                ("width", Some(at), None, None, None)
            }
            BadWidth { .. } | WidthConflict { .. } | WidthTooSmall { .. } => {
                ("width", None, None, None, None)
            }
            TooLarge { at } => ("too_large", Some(at), None, None, None),
            BadCounter { .. } => ("bad_counter", None, None, None, None),
            StarNeedsRange { at } => ("star_needs_range", Some(at), None, None, None),
            SecondStar { at } => ("second_star", Some(at), None, None, None),
            NoStar => ("no_star", None, None, None, None),
            NoPattern => ("no_pattern", None, None, None, None),
            TooMany { count, max } => ("too_many", None, None, count, Some(max)),
            InvalidUrl { .. } => ("bad_url", None, None, None, None),
        };
        BatchProblem {
            kind: kind.into(),
            at,
            text,
            count,
            max,
        }
    }
}

/// The links of `request`, by `zajil batch`'s rules.
fn links(request: &BatchRequest) -> Result<Vec<String>, PatternError> {
    let star = match (&request.from, &request.to) {
        (Some(from), Some(to)) if !from.trim().is_empty() || !to.trim().is_empty() => Some(Star {
            from: from.trim().to_owned(),
            to: to.trim().to_owned(),
            width: request.width,
        }),
        _ => None,
    };
    pattern::expand(request.pattern.trim(), star.as_ref(), MAX_URLS)
}

/// The preview: how many links, and the first [`SHOWN`].
#[tauri::command]
pub fn batch_preview(request: BatchRequest) -> Result<BatchPreview, BatchProblem> {
    let links = links(&request)?;
    // A video site's page never comes out of the engine as a file (the
    // something that lands is the site's HTML page, as the owner's
    // playlist attempt came out `watch (12).html`): the batch is stopped
    // here, and «Video quality» reads the video. The page's own list says
    // the same (`videoPage.ts`).
    let pages = links
        .iter()
        .filter(|link| crate::video_page::known_video_page(link))
        .count();
    if pages > 0 {
        return Err(BatchProblem {
            kind: "video_page".into(),
            at: None,
            text: None,
            count: Some(pages as u64),
            max: None,
        });
    }
    Ok(BatchPreview {
        count: links.len() as u64,
        shown: links.into_iter().take(SHOWN).collect(),
        max: MAX_URLS,
    })
}

/// Adds every link of `request` to `queue` in one write, started or paused,
/// into `folder` (or by category); how many were added.
#[tauri::command]
pub async fn batch_add<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    request: BatchRequest,
    queue: Option<String>,
    folder: Option<String>,
    start: bool,
    handle: tauri::AppHandle<R>,
) -> Result<usize, String> {
    if crate::in_scene(&handle) {
        return Err("a ui-test scene adds nothing".into());
    }
    let links = links(&request).map_err(|e| e.to_string())?;
    let (store, settings, system) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    let added = tokio::task::spawn_blocking(move || {
        let given =
            crate::chosen_folder(folder.as_deref(), &crate::download_dir(&settings, &system))
                .map_err(StoreError::Refused)?;
        add_all(
            &store,
            &links,
            queue.as_deref(),
            folder.is_none(),
            &given,
            start,
        )
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(added)
}

/// Adds `links` to `queue` in one write: all of them or none.
fn add_all(
    store: &QueueStore,
    links: &[String],
    queue: Option<&str>,
    categorize: bool,
    given: &std::path::Path,
    start: bool,
) -> Result<usize, StoreError> {
    store.update(|queues| {
        let queue = queue.unwrap_or(zajil_core::queue::DEFAULT_QUEUE);
        for link in links {
            let id = queues.add(queue, link, given)?;
            let item = queues.item_mut(id).expect("just added");
            item.categorize = categorize;
            if !start {
                item.status = Status::Paused;
            }
        }
        Ok(links.len())
    })
}

/// Opens «Batch download», with `url` in its pattern box if one is given.
#[tauri::command]
pub async fn open_batch<R: tauri::Runtime>(
    pending: State<'_, Pending>,
    url: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    *pending.0.lock().unwrap() = url;
    crate::windows::open(&handle, LABEL, "Zajil", SIZE).map_err(|e| e.to_string())
}

/// The link the window starts with, once.
#[tauri::command]
pub fn batch_pending(pending: State<'_, Pending>) -> Option<String> {
    pending.0.lock().unwrap().take()
}

/// «Batch download from the clipboard»: the clipboard's link, if it holds
/// one (a whole http or https link, nothing else); `null` otherwise.
#[tauri::command]
pub async fn clipboard_link() -> Option<String> {
    tokio::task::spawn_blocking(|| {
        use zajil_platform::clipboard::{Clipboard, Content, SystemClipboard};
        match SystemClipboard::new()
            .ok()?
            .text(zajil_core::clipboard::MAX_TEXT)
            .ok()?
        {
            Content::Text(text) => link_in(&text),
            _ => None,
        }
    })
    .await
    .ok()
    .flatten()
}

/// `text` if it is one http or https link, with the spaces around it gone.
fn link_in(text: &str) -> Option<String> {
    let text = text.trim();
    let lower = text.to_ascii_lowercase();
    let http = lower.starts_with("http://") || lower.starts_with("https://");
    (http && !text.is_empty() && !text.chars().any(|c| c.is_whitespace() || c.is_control()))
        .then(|| text.to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(
        pattern: &str,
        from: Option<&str>,
        to: Option<&str>,
        width: Option<u8>,
    ) -> BatchRequest {
        BatchRequest {
            pattern: pattern.into(),
            from: from.map(Into::into),
            to: to.map(Into::into),
            width,
        }
    }

    /// IDM's own: a `*` with «from», «to» and the digits, numbers or
    /// letters; and `zajil batch`'s ranges.
    #[test]
    fn a_pattern_makes_its_links() {
        let numbers = batch_preview(request(
            "https://site.com/img*.jpg",
            Some("8"),
            Some("11"),
            Some(3),
        ))
        .unwrap();
        assert_eq!(numbers.count, 4);
        assert_eq!(
            numbers.shown,
            [
                "https://site.com/img008.jpg",
                "https://site.com/img009.jpg",
                "https://site.com/img010.jpg",
                "https://site.com/img011.jpg"
            ]
        );
        assert_eq!(numbers.max, MAX_URLS);
        let letters = batch_preview(request(
            "https://site.com/*.png",
            Some("a"),
            Some("c"),
            None,
        ))
        .unwrap();
        assert_eq!(
            letters.shown,
            [
                "https://site.com/a.png",
                "https://site.com/b.png",
                "https://site.com/c.png"
            ]
        );
        let range =
            batch_preview(request("https://site.com/p[1-3].zip", None, None, None)).unwrap();
        assert_eq!(range.count, 3);
        // Empty «from» and «to» are none.
        assert!(
            batch_preview(request(
                "https://site.com/p[1-2].zip",
                Some(" "),
                Some(""),
                None
            ))
            .is_ok()
        );
    }

    /// The preview lists the first links only, and says how many in all.
    #[test]
    fn a_long_batch_is_counted_and_shown_in_part() {
        let many = batch_preview(request(
            "https://site.com/f*.bin",
            Some("1"),
            Some("900"),
            None,
        ))
        .unwrap();
        assert_eq!(many.count, 900);
        assert_eq!(many.shown.len(), SHOWN);
    }

    /// A batch of a video site's pages is refused with how many of its
    /// links are video pages (the owner's playlist attempt: its links
    /// landed as `watch (12).html` files, because a video site's page is
    /// not a file): «Video quality» reads them, not the batch.
    #[test]
    fn a_batch_of_video_pages_is_refused() {
        let refused = batch_preview(request(
            "https://www.youtube.com/watch?v=abc&index=*",
            Some("1"),
            Some("3"),
            None,
        ))
        .unwrap_err();
        assert_eq!(
            (refused.kind.as_str(), refused.count),
            ("video_page", Some(3))
        );
        // A playlist's page, too.
        assert_eq!(
            batch_preview(request(
                "https://youtube.com/playlist?list=PL1a2b3c&p=*",
                Some("1"),
                Some("2"),
                None
            ))
            .unwrap_err()
            .kind,
            "video_page"
        );
        // The batch of files beside a video site's page stays: only what
        // the pattern makes is asked.
        assert!(
            batch_preview(request(
                "https://site.com/img*.jpg",
                Some("8"),
                Some("11"),
                Some(3)
            ))
            .is_ok()
        );
    }

    /// A wrong pattern is refused, with what the page needs to say why.
    #[test]
    fn a_wrong_pattern_says_why() {
        let kind = |r: BatchRequest| batch_preview(r).unwrap_err().kind;
        assert_eq!(
            kind(request("ftp://site.com/*.jpg", Some("1"), Some("2"), None)),
            "not_http"
        );
        assert_eq!(
            kind(request("https://site.com/*.jpg", None, None, None)),
            "star_needs_range"
        );
        assert_eq!(
            kind(request("https://site.com/a.jpg", None, None, None)),
            "no_pattern"
        );
        // A number and a letter: mixed; not a number nor a letter: bad.
        assert_eq!(
            kind(request(
                "https://site.com/*.jpg",
                Some("1"),
                Some("b"),
                None
            )),
            "mixed"
        );
        assert_eq!(
            kind(request(
                "https://site.com/*.jpg",
                Some("1.5"),
                Some("3"),
                None
            )),
            "bad_counter"
        );
        assert_eq!(
            kind(request("https://site.com/[3-1].jpg", None, None, None)),
            "descending"
        );
        let unclosed =
            batch_preview(request("https://site.com/[1-3.jpg", None, None, None)).unwrap_err();
        assert_eq!(
            (
                unclosed.kind.as_str(),
                unclosed.at,
                unclosed.text.as_deref()
            ),
            ("unclosed", Some(18), Some("["))
        );
        let many = batch_preview(request(
            "https://site.com/*.jpg",
            Some("1"),
            Some("5000"),
            None,
        ))
        .unwrap_err();
        assert_eq!(
            (many.kind.as_str(), many.count, many.max),
            ("too_many", Some(5000), Some(MAX_URLS))
        );
    }

    /// Every link goes into the queue in one write, started or paused; a
    /// link the queues refuse (a password in it) adds none of them.
    #[test]
    fn a_batch_is_added_whole_or_not_at_all() {
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let links = links(&request(
            "https://site.com/f*.bin",
            Some("1"),
            Some("3"),
            None,
        ))
        .unwrap();
        assert_eq!(
            add_all(&store, &links, Some("night"), true, data.path(), false).unwrap(),
            3
        );
        let queues = store.load().unwrap();
        let night = &queues.queue("night").unwrap().items;
        assert_eq!(night.len(), 3);
        assert!(
            night
                .iter()
                .all(|i| i.status == Status::Paused && i.categorize)
        );
        assert_eq!(night[2].url, "https://site.com/f3.bin");
        add_all(&store, &links, None, false, data.path(), true).unwrap();
        let queues = store.load().unwrap();
        assert!(
            queues
                .queue("main")
                .unwrap()
                .items
                .iter()
                .all(|i| i.status == Status::Waiting)
        );
        let bad = vec![
            "https://site.com/a".to_owned(),
            "https://u:p@site.com/b".to_owned(),
        ];
        assert!(add_all(&store, &bad, None, true, data.path(), true).is_err());
        assert_eq!(store.load().unwrap().queue("main").unwrap().items.len(), 3);
    }

    /// The window may listen, set its title and pick a folder; nothing
    /// more (no file system, no shell, no network from the page).
    #[test]
    fn the_batch_window_has_only_its_own_permissions() {
        let capability: serde_json::Value = serde_json::from_str(CAPABILITY).unwrap();
        assert_eq!(capability["windows"], serde_json::json!([LABEL]));
        assert_eq!(
            capability["permissions"],
            serde_json::json!([
                "core:event:default",
                "core:window:allow-set-title",
                "dialog:allow-open"
            ])
        );
    }

    #[test]
    fn only_a_whole_link_is_taken_from_the_clipboard() {
        assert_eq!(
            link_in("  https://site.com/img*.jpg \n").as_deref(),
            Some("https://site.com/img*.jpg")
        );
        for not in [
            "",
            "hello",
            "see https://site.com/a.jpg",
            "file:///etc/passwd",
            "https://a.com/x b",
        ] {
            assert_eq!(link_in(not), None, "{not}");
        }
    }
}
