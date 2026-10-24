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
use zajil_core::category::CategoriesFile;
use zajil_core::queue::{Item, QueuesFile, Status};
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

/// The name the engine would give a download of `url` before the server
/// answers (no `Content-Disposition` yet): the same function it uses.
fn name_from_url(url: &str) -> String {
    zajil_core::Url::parse(url)
        .map(|u| zajil_core::naming::suggested_name(None, &u))
        .unwrap_or_else(|_| "download".to_owned())
}

fn host_of(url: &str) -> Option<String> {
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
        category: categories
            .classify(&name, host.as_deref())
            .map(|c| c.id.clone()),
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
        categories: categories
            .categories
            .iter()
            .map(|c| CategoryView {
                id: c.id.clone(),
                name: c.name.clone(),
                extensions: c.extensions.clone(),
                sites: c.sites.clone(),
                folder: c.folder.as_ref().map(|f| f.display().to_string()),
            })
            .collect(),
        speed_limit: queues.settings.speed_limit,
        problem: live.problem(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

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
}
