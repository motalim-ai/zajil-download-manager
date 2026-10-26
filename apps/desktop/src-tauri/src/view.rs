//! What the page receives and sends. The TypeScript types are generated
//! from these (ts-rs, into `apps/desktop/src/lib/bindings/`) by
//! `cargo test`, and CI checks that the generated files are up to date, so
//! the two sides cannot drift apart (ADR-0003).
//!
//! Sizes are numbers in TypeScript: a file would have to pass 8 PiB to
//! lose precision in a JavaScript number.

use std::path::Path;

use serde::{Deserialize, Serialize};
use ts_rs::TS;
use zajil_core::Probe;
use zajil_core::category::{CategoriesFile, Matched};
use zajil_core::queue::{ImportSummary, Item, QueuesFile, Status};
use zajil_core::settings::{Digits, Language, SettingsFile, Theme};
use zajil_core::state::{DownloadState, STATE_SUFFIX};

use crate::live::Live;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum StatusView {
    Waiting,
    Active,
    Paused,
    Done,
    Failed,
}

impl From<Status> for StatusView {
    fn from(s: Status) -> Self {
        match s {
            Status::Waiting => StatusView::Waiting,
            Status::Active => StatusView::Active,
            Status::Paused => StatusView::Paused,
            Status::Done => StatusView::Done,
            Status::Failed => StatusView::Failed,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DownloadView {
    #[ts(type = "number")]
    pub id: u64,
    /// The file's name: the finished file, the one being written, or the
    /// last part of the URL before the server has answered.
    pub name: String,
    /// The host of the URL (never the query, which may hold a token).
    pub host: Option<String>,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    #[ts(type = "number")]
    pub done: u64,
    /// Bytes per second now (0 when not running).
    pub speed: f64,
    pub status: StatusView,
    pub queue: String,
    /// The category id, from the name (and host).
    pub category: Option<String>,
    /// Where it is saved (the folder).
    pub folder: String,
    /// How far each feather is, from 0 to 1, in file order: the file in at
    /// most eight equal pieces.
    pub parts: Vec<f64>,
    /// How many segments the file is in now: one per connection at first,
    /// then more as they split.
    pub segments: u32,
    pub error: Option<String>,
    /// It waits for its browser's cookies after a restart, and that
    /// browser is not open (M5): «افتح المتصفح ليكمل».
    pub waits_for_browser: bool,
    /// Next in its queue, its site has no free connection (the user's
    /// per-site limit, #170): «بانتظار الموقع».
    pub waits_for_site: bool,
    /// The user's description (#95).
    pub description: Option<String>,
    /// When it last started or resumed (#95): RFC 3339 in UTC.
    pub last_try: Option<String>,
    /// A file that can run code, from the browser: it waits for the user
    /// to start it (security review #84).
    pub needs_ok: bool,
    /// A video download (#149): no queue, no parts; «pause» stops it, and
    /// it starts over.
    pub video: bool,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct QueueView {
    pub name: String,
    pub max_active: u32,
    pub active: u32,
    /// The daily window, `HH:MM` to `HH:MM`, if the queue has a schedule.
    pub window: Option<(String, String)>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct CategoryView {
    pub id: String,
    /// The user's name for it; `null` for a built-in category that the
    /// page names in the user's language (`category-<id>`).
    pub name: Option<String>,
    pub extensions: Vec<String>,
    pub sites: Vec<String>,
    pub folder: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct Snapshot {
    pub downloads: Vec<DownloadView>,
    pub queues: Vec<QueueView>,
    pub categories: Vec<CategoryView>,
    #[ts(type = "number | null")]
    pub speed_limit: Option<u64>,
    /// Why the queues cannot run now (a damaged queues file), if so.
    pub problem: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ProbeView {
    pub name: String,
    #[ts(type = "number | null")]
    pub size: Option<u64>,
    pub resumable: bool,
    /// The server announced ranges (without a size or a strong validator,
    /// it still cannot be resumed: the dialog says which).
    pub ranges: bool,
    pub category: Option<String>,
    /// What matched the category: one of its sites, or else the extension.
    pub category_site: Option<String>,
    pub category_extension: Option<String>,
    /// The folder it would go to without a folder of its own.
    pub folder: String,
    /// The server sent a web page, not a file (a Google Drive or sign-in
    /// page): «Download file info» warns.
    pub page: bool,
    /// The downloads folder: where a category with no folder of its own
    /// saves (the window's folder follows the category the user picks).
    pub download_folder: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct AddRequest {
    pub url: String,
    /// The name the user gave the file; `null`: the server's (#103).
    #[serde(default)]
    pub name: Option<String>,
    /// The category the user chose; `null`: by its name or site (#103).
    #[serde(default)]
    pub category: Option<String>,
    /// The user's description (#95).
    #[serde(default)]
    pub description: Option<String>,
    pub queue: Option<String>,
    /// A folder of its own; `null`: by category.
    pub folder: Option<String>,
    /// Start when its queue has room; `false`: added paused.
    pub start: bool,
    #[ts(type = "number | null")]
    pub limit: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SegmentView {
    #[ts(type = "number")]
    pub start: u64,
    /// The last byte of the part, or `null` when the size is unknown.
    #[ts(type = "number | null")]
    pub end: Option<u64>,
    #[ts(type = "number")]
    pub done: u64,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct DetailsView {
    pub download: DownloadView,
    /// The link, whole (the progress window shows it, as the add windows
    /// do; never printed in a log).
    pub url: String,
    pub segments: Vec<SegmentView>,
    /// `null` when unknown: no state file (finished, or not started yet).
    pub resumable: Option<bool>,
    #[ts(type = "number | null")]
    pub limit: Option<u64>,
}

/// Why a download did not take a new link (M5, "refresh the download
/// link"), for the page to say in the user's language.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(tag = "kind", rename_all = "snake_case")]
#[ts(export)]
pub enum RefreshProblem {
    /// It is downloading or waiting for its turn: pause it first.
    NotStopped,
    /// Paused a moment ago, it is still stopping (it holds its part file):
    /// try again in a moment (review of #78).
    Stopping,
    /// Another refresh of it is under way.
    Busy,
    /// Given up: the dialog was closed.
    Cancelled,
    /// It has finished.
    Done,
    /// Not a link Zajil takes; the reason, in English.
    BadLink { reason: String },
    /// The server does not show it is the same file.
    NotTheSameFile {
        why: MismatchView,
        #[ts(type = "number | null")]
        theirs: Option<u64>,
        #[ts(type = "number | null")]
        ours: Option<u64>,
    },
    /// Anything else (the new link's server refused, no connection…); the
    /// engine's message, in English.
    Failed { error: String },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum MismatchView {
    NoValidator,
    Size,
    ValidatorKind,
    NewNoValidator,
    Version,
}

impl From<zajil_core::Error> for RefreshProblem {
    fn from(err: zajil_core::Error) -> Self {
        use zajil_core::{Error, Mismatch};
        match err {
            Error::NotTheSameFile(mismatch) => {
                let (why, theirs, ours) = match mismatch {
                    Mismatch::NoValidator => (MismatchView::NoValidator, None, None),
                    Mismatch::Size { theirs, ours } => (MismatchView::Size, theirs, ours),
                    Mismatch::ValidatorKind => (MismatchView::ValidatorKind, None, None),
                    Mismatch::NewNoValidator => (MismatchView::NewNoValidator, None, None),
                    Mismatch::Version => (MismatchView::Version, None, None),
                };
                RefreshProblem::NotTheSameFile { why, theirs, ours }
            }
            Error::Locked(_) => RefreshProblem::Stopping,
            Error::Stopped { .. } => RefreshProblem::Cancelled,
            err @ (Error::InvalidUrl { .. } | Error::UnsupportedScheme(_)) => {
                RefreshProblem::BadLink {
                    reason: err.to_string(),
                }
            }
            err => RefreshProblem::Failed {
                error: err.to_string(),
            },
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum LanguageView {
    Ar,
    En,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum ThemeView {
    System,
    Light,
    Dark,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum DigitsView {
    Latn,
    Arab,
}

/// The settings screen: what the user chose, and what the choices mean.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsView {
    pub language: LanguageView,
    pub theme: ThemeView,
    pub digits: DigitsView,
    /// The folder chosen; `null`: the system's downloads folder.
    pub download_dir: Option<String>,
    /// The system's downloads folder, shown when none is chosen.
    pub system_download_dir: String,
    pub connections: u32,
    pub max_connections: u32,
    /// A notification when a download finishes or fails.
    pub notifications: bool,
    /// Clipboard monitoring: a copied download link opens the add dialog.
    pub clipboard: bool,
    /// Look for a newer Zajil at start (#124).
    pub update_check: bool,
    /// Bytes per second for all downloads together; `null`: no limit.
    #[ts(type = "number | null")]
    pub speed_limit: Option<u64>,
}

/// What the settings screen saves.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct SettingsChange {
    pub language: LanguageView,
    pub theme: ThemeView,
    pub digits: DigitsView,
    pub download_dir: Option<String>,
    pub connections: u32,
    pub notifications: bool,
    pub clipboard: bool,
    pub update_check: bool,
    #[ts(type = "number | null")]
    pub speed_limit: Option<u64>,
}

/// The browser integration (M5): `zajil-host` registered with each
/// browser, or why not.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct IntegrationView {
    /// Why nothing was registered; `null`: registered (see `browsers`).
    pub host_problem: Option<HostProblemView>,
    pub browsers: Vec<BrowserIntegrationView>,
}

/// Why `zajil-host` next to the program was not registered.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, TS)]
#[serde(rename_all = "snake_case")]
#[ts(export)]
pub enum HostProblemView {
    /// Not there, or not a program (a development build).
    Missing,
    /// The program runs from a place that goes away when it closes (an
    /// AppImage's mount, macOS's App Translocation): a browser would later
    /// start whatever another user put at that path.
    Temporary,
    /// Another user could change it: its folder or itself is writable by
    /// others, or owned by someone else.
    Unsafe,
}

/// One browser's registration.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct BrowserIntegrationView {
    /// `chrome`, `chromium`, `edge` or `firefox`.
    pub browser: String,
    /// Why it failed; `null`: registered.
    pub error: Option<String>,
}

/// What an import did.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct ImportView {
    pub items_added: u32,
    pub queues_created: u32,
    /// Queues that already had settings of their own, and kept them.
    pub kept_settings: Vec<String>,
    /// The file's speed limit was taken (there was none).
    pub speed_limit_set: bool,
}

impl From<&ImportSummary> for ImportView {
    fn from(s: &ImportSummary) -> Self {
        ImportView {
            items_added: u32::try_from(s.items_added).unwrap_or(u32::MAX),
            queues_created: u32::try_from(s.queues_created).unwrap_or(u32::MAX),
            kept_settings: s.kept_settings.clone(),
            speed_limit_set: s.speed_limit_set,
        }
    }
}

pub fn settings(file: &SettingsFile, speed_limit: Option<u64>, system_dir: &Path) -> SettingsView {
    SettingsView {
        language: match file.language {
            Language::Ar => LanguageView::Ar,
            Language::En => LanguageView::En,
        },
        theme: match file.theme {
            Theme::System => ThemeView::System,
            Theme::Light => ThemeView::Light,
            Theme::Dark => ThemeView::Dark,
        },
        digits: match file.digits {
            Digits::Latn => DigitsView::Latn,
            Digits::Arab => DigitsView::Arab,
        },
        download_dir: file.download_dir.as_ref().map(|d| d.display().to_string()),
        system_download_dir: system_dir.display().to_string(),
        connections: file.connections,
        max_connections: zajil_core::MAX_CONNECTIONS as u32,
        notifications: file.notifications,
        clipboard: file.clipboard,
        update_check: file.update_check,
        speed_limit,
    }
}

/// `change` applied to `file` (the speed limit goes to the queues file).
pub fn apply_settings(file: &mut SettingsFile, change: &SettingsChange) {
    file.language = match change.language {
        LanguageView::Ar => Language::Ar,
        LanguageView::En => Language::En,
    };
    file.theme = match change.theme {
        ThemeView::System => Theme::System,
        ThemeView::Light => Theme::Light,
        ThemeView::Dark => Theme::Dark,
    };
    file.digits = match change.digits {
        DigitsView::Latn => Digits::Latn,
        DigitsView::Arab => Digits::Arab,
    };
    file.download_dir = change.download_dir.as_ref().map(std::path::PathBuf::from);
    file.connections = change.connections;
    file.notifications = change.notifications;
    file.clipboard = change.clipboard;
    file.update_check = change.update_check;
}

/// The categories as the page edits them, back into categories. Every
/// value is checked when the file is written (`CategoryStore::update`).
pub fn categories_from(views: &[CategoryView]) -> Vec<zajil_core::category::Category> {
    views
        .iter()
        .map(|v| zajil_core::category::Category {
            id: v.id.clone(),
            name: v.name.clone(),
            extensions: v.extensions.clone(),
            sites: v.sites.clone(),
            folder: v.folder.as_ref().map(std::path::PathBuf::from),
        })
        .collect()
}

pub fn category_views(categories: &CategoriesFile) -> Vec<CategoryView> {
    categories
        .categories
        .iter()
        .map(|c| CategoryView {
            id: c.id.clone(),
            name: c.name.clone(),
            extensions: c.extensions.clone(),
            sites: c.sites.clone(),
            folder: c.folder.as_ref().map(|f| f.display().to_string()),
        })
        .collect()
}

/// The name the engine would give a download of `url` before the server
/// answers (no `Content-Disposition` yet): the same function it uses
/// (the engine may then add the extension of the server's type).
fn name_from_url(url: &str) -> String {
    zajil_core::Url::parse(url)
        .map(|u| zajil_core::naming::suggested_name(None, &u))
        .unwrap_or_else(|_| "download".to_owned())
}

pub(crate) fn host_of(url: &str) -> Option<String> {
    zajil_core::Url::parse(url)
        .ok()
        .and_then(|u| u.host_str().map(str::to_owned))
}

fn file_name(path: &Path) -> Option<String> {
    path.file_name().map(|n| n.to_string_lossy().into_owned())
}

/// Feathers drawn for a download at most: more would be too thin to read.
const FEATHERS: usize = 8;

/// How far each feather is, from 0 to 1, in file order: the file cut into
/// as many equal pieces as it has segments, at most [`FEATHERS`], each
/// filled by its bytes on disk. Segments split as the download runs, so
/// there are often more than eight, of unequal sizes (review of #46: taking
/// the first eight drew eight full feathers at 50%).
fn parts(state: Option<&DownloadState>, status: Status) -> Vec<f64> {
    let whole = if status == Status::Done { 1.0 } else { 0.0 };
    let Some(segments) = state.map(|s| s.segments.as_slice()) else {
        return vec![whole];
    };
    // The file's size; unknown (a single segment with no end): one feather.
    let size = segments
        .iter()
        .map(|s| s.end.map(|e| e.saturating_add(1)))
        .collect::<Option<Vec<_>>>()
        .and_then(|ends| ends.into_iter().max());
    let Some(size) = size.filter(|&size| size > 0) else {
        return vec![whole];
    };
    let pieces = segments.len().clamp(1, FEATHERS) as u128;
    (0..pieces)
        .map(|i| {
            let lo = (u128::from(size) * i / pieces) as u64;
            let hi = (u128::from(size) * (i + 1) / pieces) as u64;
            let on_disk = segments.iter().fold(0u64, |sum, s| {
                let written = s
                    .start
                    .saturating_add(s.done)
                    .min(s.end.map_or(u64::MAX, |e| e.saturating_add(1)));
                sum.saturating_add(written.min(hi).saturating_sub(s.start.max(lo)))
            });
            if hi == lo {
                whole
            } else {
                (on_disk as f64 / (hi - lo) as f64).min(1.0)
            }
        })
        .collect()
}

pub fn download(
    item: &Item,
    queue: &str,
    categories: &CategoriesFile,
    live: &Live,
) -> DownloadView {
    let now = live.item(item.id);
    let state_file = now
        .as_ref()
        .and_then(|l| l.state_file.clone())
        .or_else(|| item.state_file.clone());
    let state = state_file
        .as_deref()
        .and_then(|p| DownloadState::load(p).ok());
    let name = item
        .file
        .as_deref()
        .and_then(file_name)
        .or_else(|| {
            now.as_ref()
                .and_then(|l| l.file.as_deref())
                .and_then(file_name)
        })
        .or_else(|| {
            state_file
                .as_deref()
                .and_then(file_name)
                .and_then(|n| n.strip_suffix(STATE_SUFFIX).map(str::to_owned))
        })
        // The user's own name (#103) before the server has been asked.
        .or_else(|| item.name.clone())
        .unwrap_or_else(|| name_from_url(&item.url));
    let host = host_of(&item.url);
    let folder = item
        .file
        .as_deref()
        .or_else(|| now.as_ref().and_then(|l| l.file.as_deref()))
        .and_then(Path::parent)
        .map(Path::to_path_buf)
        .unwrap_or_else(|| item.dir.clone());
    let (done, size, speed) = match (&now, &state) {
        (Some(l), _) => (
            l.done,
            l.total.or(state.as_ref().and_then(|s| s.content_length)),
            l.speed,
        ),
        (None, Some(s)) => (s.bytes_done(), s.content_length, 0.0),
        (None, None) => (0, None, 0.0),
    };
    let done = if item.status == Status::Done {
        size.unwrap_or(done)
    } else {
        done
    };
    DownloadView {
        id: item.id,
        // The category the user chose (#103), while it exists; else the
        // one its name or site gives.
        category: item
            .category
            .as_deref()
            .and_then(|id| categories.get(id))
            .or_else(|| categories.classify(&name, host.as_deref()))
            .map(|c| c.id.clone()),
        description: item.description.clone(),
        last_try: item.last_try.clone(),
        name,
        host,
        size,
        done,
        speed,
        status: item.status.into(),
        queue: queue.to_owned(),
        folder: folder.display().to_string(),
        parts: parts(state.as_ref(), item.status),
        segments: state.as_ref().map_or(1, |s| {
            u32::try_from(s.segments.len().max(1)).unwrap_or(u32::MAX)
        }),
        error: item.error.clone(),
        waits_for_browser: item.status == Status::Waiting && live.waiting_for_browser(item.id),
        waits_for_site: item.status == Status::Waiting && live.waiting_for_site(item.id),
        needs_ok: item.needs_ok && item.status == Status::Paused,
        video: false,
    }
}

pub fn snapshot(queues: &QueuesFile, categories: &CategoriesFile, live: &Live) -> Snapshot {
    let downloads = queues
        .queues
        .iter()
        .flat_map(|q| q.items.iter().map(move |i| (q, i)))
        .map(|(q, i)| download(i, &q.name, categories, live))
        .collect();
    Snapshot {
        downloads,
        queues: queues
            .queues
            .iter()
            .map(|q| QueueView {
                name: q.name.clone(),
                max_active: q.max_active,
                active: q
                    .items
                    .iter()
                    .filter(|i| i.status == Status::Active)
                    .count() as u32,
                window: q.schedule.as_ref().map(|s| {
                    (
                        s.start
                            .map_or_else(|| "00:00".to_owned(), |t| t.to_string()),
                        s.stop.map_or_else(|| "24:00".to_owned(), |t| t.to_string()),
                    )
                }),
            })
            .collect(),
        categories: category_views(categories),
        speed_limit: queues.settings.speed_limit,
        problem: live.problem(),
    }
}

pub fn probe(found: &Probe, categories: &CategoriesFile, given: &Path) -> ProbeView {
    let matched = categories.classify_why(&found.name, found.host.as_deref());
    let (category_site, category_extension) = match matched.as_ref().map(|(_, m)| m) {
        Some(Matched::Site(site)) => (Some(site.clone()), None),
        Some(Matched::Extension(ext)) => (None, Some(ext.clone())),
        None => (None, None),
    };
    ProbeView {
        name: found.name.clone(),
        size: found.size,
        resumable: found.resumable,
        ranges: found.ranges,
        category: matched.map(|(c, _)| c.id.clone()),
        category_site,
        category_extension,
        folder: categories
            .folder_for(&found.name, found.host.as_deref(), given)
            .display()
            .to_string(),
        page: found.is_page(),
        download_folder: given.display().to_string(),
    }
}

/// The details of one download, with the same categories and queue name
/// as in the list (review of #48: the built-in categories only, and no
/// queue name).
pub fn details(
    item: &Item,
    queue: &str,
    categories: &CategoriesFile,
    state: Option<&DownloadState>,
    live: &Live,
) -> DetailsView {
    let download = download(item, queue, categories, live);
    DetailsView {
        segments: state
            .map(|s| {
                s.segments
                    .iter()
                    .map(|g| SegmentView {
                        start: g.start,
                        end: g.end,
                        done: g.done,
                    })
                    .collect()
            })
            .unwrap_or_default(),
        // The same rule as the add dialog (`Engine::probe`).
        resumable: state.map(DownloadState::resumable),
        limit: item.limit,
        url: item.url.clone(),
        download,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// «Download file info» gets the name the file is saved under and
    /// whether it is a web page (Google Drive answers a page, review of the
    /// owner's test).
    #[test]
    fn the_info_window_gets_the_name_and_whether_it_is_a_web_page() {
        let mut categories = CategoriesFile::built_in();
        categories.categories.clear();
        let found = Probe {
            name: "download".into(),
            size: None,
            resumable: false,
            ranges: false,
            host: Some("drive.usercontent.google.com".into()),
            content_type: Some("text/html".into()),
        };
        let view = probe(&found, &categories, Path::new("/d"));
        assert_eq!(view.name, "download");
        assert_eq!(view.download_folder, "/d");
        assert_eq!(view.size, None);
        assert!(view.page);
        let file = Probe {
            name: "a.zip".into(),
            content_type: Some("application/zip".into()),
            ..found
        };
        assert!(!probe(&file, &categories, Path::new("/d")).page);
        let xhtml = Probe {
            content_type: Some("application/xhtml+xml".into()),
            ..file.clone()
        };
        assert!(probe(&xhtml, &categories, Path::new("/d")).page);
        let unknown = Probe {
            content_type: None,
            ..file
        };
        assert!(!probe(&unknown, &categories, Path::new("/d")).page);
    }

    /// AGENTS.md §10.3: what the page receives never holds a URL's query
    /// (a token) or its user name and password, even if an item had them.
    #[test]
    fn the_page_never_receives_a_token_or_a_password() {
        let mut queues = QueuesFile::empty();
        let dir = std::env::temp_dir();
        queues
            .add(
                "main",
                "https://cdn.example.net/file.iso?sig=SECRET-TOKEN&e=1",
                &dir,
            )
            .unwrap();
        // Written by hand: `add` refuses credentials, an old file might not.
        let mut odd = queues.item(1).unwrap().clone();
        odd.id = 2;
        odd.url = "https://user:hunter2@cdn.example.net/b.zip?token=OTHER-SECRET".into();
        queues.queues[0].items.push(odd);
        let snapshot = snapshot(&queues, &CategoriesFile::built_in(), &Live::default());
        let json = serde_json::to_string(&snapshot).unwrap();
        for secret in [
            "SECRET-TOKEN",
            "OTHER-SECRET",
            "hunter2",
            "user:",
            "sig=",
            "token=",
        ] {
            assert!(!json.contains(secret), "{secret} in {json}");
        }
        assert_eq!(
            snapshot.downloads[0].host.as_deref(),
            Some("cdn.example.net")
        );
        assert_eq!(snapshot.downloads[0].name, "file.iso");
        assert_eq!(snapshot.downloads[1].name, "b.zip");
    }

    fn segment(start: u64, end: u64, done: u64) -> zajil_core::state::Segment {
        zajil_core::state::Segment {
            start,
            end: Some(end),
            done,
        }
    }

    /// Review of #46: sixteen segments, the first eight whole and the last
    /// eight empty, are half the file: four full feathers and four empty,
    /// not the first eight segments drawn full.
    #[test]
    fn the_feathers_show_the_file_in_equal_pieces_whatever_its_segments() {
        let mut state = DownloadState::new(
            "https://example.net/f.bin".into(),
            "https://example.net/f.bin".into(),
            zajil_core::state::Validator {
                etag: Some("\"v1\"".into()),
                last_modified: None,
                date: None,
            },
            Some(1600),
            true,
        );
        state.segments = (0..16)
            .map(|i| segment(i * 100, i * 100 + 99, if i < 8 { 100 } else { 0 }))
            .collect();
        assert_eq!(
            parts(Some(&state), Status::Active),
            [1.0, 1.0, 1.0, 1.0, 0.0, 0.0, 0.0, 0.0]
        );
        // Unequal segments after a split: the bytes count, not the segments.
        state.segments = vec![
            segment(0, 599, 600),
            segment(600, 749, 75),
            segment(750, 899, 0),
        ];
        assert_eq!(parts(Some(&state), Status::Active), [1.0, 1.0, 0.25]);
        // Eight equal segments: one feather each, as at the start.
        state.segments = (0..8).map(|i| segment(i * 10, i * 10 + 9, i)).collect();
        let feathers = parts(Some(&state), Status::Active);
        assert_eq!(feathers.len(), 8);
        for (i, f) in feathers.iter().enumerate() {
            assert!((f - i as f64 / 10.0).abs() < 1e-9, "{feathers:?}");
        }
        // Of unknown size: one feather, full when done.
        state.segments = vec![zajil_core::state::Segment {
            start: 0,
            end: None,
            done: 5,
        }];
        assert_eq!(parts(Some(&state), Status::Active), [0.0]);
        assert_eq!(parts(Some(&state), Status::Done), [1.0]);
        assert_eq!(parts(None, Status::Waiting), [0.0]);
    }

    fn state_with(etag: Option<&str>, size: Option<u64>, ranges: bool) -> DownloadState {
        DownloadState::new(
            "https://example.net/f.iso".into(),
            "https://example.net/f.iso".into(),
            zajil_core::state::Validator {
                etag: etag.map(str::to_owned),
                last_modified: None,
                date: None,
            },
            size,
            ranges,
        )
    }

    /// Review of #48: the details show the download as the list does (the
    /// user's categories, its queue), and call it resumable by the add
    /// dialog's rule, not on `Accept-Ranges` alone.
    #[test]
    fn the_details_agree_with_the_list_and_the_add_dialog() {
        let mut queues = QueuesFile::empty();
        let dir = std::env::temp_dir();
        queues
            .add("night", "https://isos.example.net/f.iso", &dir)
            .unwrap();
        let mut categories = CategoriesFile::built_in();
        categories.categories.insert(
            0,
            zajil_core::category::Category {
                id: "isos".into(),
                name: Some("ISOs".into()),
                extensions: vec!["iso".into()],
                sites: vec![],
                folder: None,
            },
        );
        let item = queues.item(1).unwrap();
        let strong = state_with(Some("\"v1\""), Some(100), true);
        let shown = details(item, "night", &categories, Some(&strong), &Live::default());
        assert_eq!(shown.download.queue, "night");
        assert_eq!(shown.download.category.as_deref(), Some("isos"));
        assert_eq!(shown.resumable, Some(true));
        for (state, why) in [
            (state_with(Some("W/\"v1\""), Some(100), true), "a weak ETag"),
            (state_with(Some("\"v1\""), None, true), "no size"),
            (state_with(Some("\"v1\""), Some(100), false), "no ranges"),
        ] {
            let shown = details(item, "night", &categories, Some(&state), &Live::default());
            assert_eq!(shown.resumable, Some(false), "{why}");
        }
        // Without a state file it is not known (review of #48: it said No).
        assert_eq!(
            details(item, "night", &categories, None, &Live::default()).resumable,
            None
        );
    }

    /// #124: the settings screen's switch saves the update check.
    #[test]
    fn the_update_check_switch_is_saved() {
        let mut file = SettingsFile::default();
        let shown = settings(&file, None, Path::new("/d"));
        assert!(shown.update_check);
        let change = SettingsChange {
            language: shown.language,
            theme: shown.theme,
            digits: shown.digits,
            download_dir: None,
            connections: shown.connections,
            notifications: shown.notifications,
            clipboard: shown.clipboard,
            update_check: false,
            speed_limit: None,
        };
        apply_settings(&mut file, &change);
        assert!(!file.update_check);
        assert!(!settings(&file, None, Path::new("/d")).update_check);
    }
}
