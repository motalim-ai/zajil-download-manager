//! «Site grabber» (M7, ADR-0010, as IDM's Site Grabber): a start page, the
//! depth, the kinds of files and the other sites allowed, then «Check»
//! reads the site with `zajil grab`'s own crawl (`zajil_grab::crawl`, a dry
//! run: pages only, no file downloaded) and lists the files it would get.
//! The user takes some out, then they go into a queue of the site's own,
//! one download at a time (owner's decision of 2026-10-26: the site's
//! manners, as `zajil grab`), with one connection each (#211). robots.txt
//! is always respected (ADR-0010, owner's decision 2: turning it off is
//! the command line's only).
//!
//! The list stays here: the page shows it and says which ones it took out,
//! by their place in it; the links themselves never come back from it.

use std::sync::Mutex;
use std::sync::atomic::{AtomicU64, Ordering};

use serde::{Deserialize, Serialize};
use tauri::{Emitter, State};
use ts_rs::TS;
use zajil_core::queue::{QueueStore, QueuesFile, Status, StoreError};
use zajil_core::{Stop, Url};
use zajil_grab::crawl::{self, Event, Plan, RobotsOutcome};
use zajil_grab::scope::{AllowedHost, Scope};

pub const LABEL: &str = "grab";
pub const SIZE: (f64, f64) = (720.0, 680.0);

/// The window's capability: events, its title, and the folder picker.
pub const CAPABILITY: &str = include_str!("../runtime-capabilities/grab.json");

/// The kinds of files the window offers, and their extensions: `zajil
/// grab`'s own defaults (archives, documents, audio, video, programs), and
/// images, which it leaves out unless asked.
pub const KINDS: &[(&str, &[&str])] = &[
    (
        "archives",
        &[
            "zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst", "iso", "dmg",
        ],
    ),
    (
        "documents",
        &[
            "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp", "epub", "rtf",
            "txt", "csv",
        ],
    ),
    (
        "audio",
        &["mp3", "m4a", "aac", "flac", "wav", "ogg", "opus"],
    ),
    (
        "video",
        &["mp4", "mkv", "avi", "mov", "webm", "m4v", "wmv", "flv"],
    ),
    (
        "programs",
        &["exe", "msi", "apk", "deb", "rpm", "appimage", "pkg"],
    ),
    (
        "images",
        &["jpg", "jpeg", "png", "gif", "webp", "bmp", "tif", "tiff"],
    ),
];

/// What to grab, as the window asks.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, TS)]
#[ts(export)]
pub struct GrabRequest {
    pub start: String,
    pub depth: u8,
    /// Kinds from [`KINDS`].
    pub kinds: Vec<String>,
    /// Extensions the user wrote, with or without the dot.
    pub extensions: Vec<String>,
    /// Other sites, `HOST` or `HOST:PORT`, allowed besides the start's.
    pub sites: Vec<String>,
    #[ts(type = "number")]
    pub max_files: u64,
    /// Bytes in all.
    #[ts(type = "number")]
    pub max_total: u64,
}

/// What a check found.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct GrabScan {
    /// Which check this is: adding names it.
    #[ts(type = "number")]
    pub id: u64,
    pub files: Vec<String>,
    #[ts(type = "number")]
    pub pages: u64,
    /// Links left out (outside the site, not a kind asked for, robots.txt…).
    #[ts(type = "number")]
    pub skipped: u64,
    /// Limits reached: `files`, `pages`, `total`, `waiting`, `seen`.
    pub limits: Vec<String>,
    /// robots.txt allowed nothing on the start's site (or it could not be
    /// read: then nothing is allowed).
    pub robots_closed: bool,
    pub stopped: bool,
}

/// How far a check is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct GrabProgress {
    #[ts(type = "number")]
    pub pages: u64,
    #[ts(type = "number")]
    pub files: u64,
}

#[derive(Default)]
pub struct GrabState {
    next: AtomicU64,
    /// The last check: its id, its site, and its files.
    last: Mutex<Option<(u64, String, Vec<String>)>>,
    running: Mutex<Option<Stop>>,
    /// The link the window starts with.
    pending: Mutex<Option<String>>,
}

/// The extensions of `kinds` and of what the user wrote, lowercase, once
/// each.
fn types_of(kinds: &[String], extensions: &[String]) -> Vec<String> {
    let mut types: Vec<String> = Vec::new();
    let wanted = KINDS
        .iter()
        .filter(|(kind, _)| kinds.iter().any(|k| k == kind))
        .flat_map(|(_, exts)| exts.iter().map(|e| (*e).to_owned()));
    let written = extensions
        .iter()
        .map(|e| e.trim().trim_start_matches('.').to_ascii_lowercase())
        .filter(|e| !e.is_empty() && e.len() <= 10 && e.bytes().all(|b| b.is_ascii_alphanumeric()));
    for ext in wanted.chain(written) {
        if !types.contains(&ext) {
            types.push(ext);
        }
    }
    types
}

/// The plan and scope of `request`, checked: `zajil grab`'s rules and
/// limits; robots.txt always respected.
fn plan_of(request: &GrabRequest) -> Result<(Plan, Scope), String> {
    let start = Url::parse(request.start.trim()).map_err(|_| "start".to_owned())?;
    let allowed = request
        .sites
        .iter()
        .map(|s| s.trim())
        .filter(|s| !s.is_empty())
        .map(|s| s.parse::<AllowedHost>().map_err(|_| format!("site:{s}")))
        .collect::<Result<Vec<_>, _>>()?;
    let scope = Scope::new(&start, allowed.clone(), false).map_err(|_| "start".to_owned())?;
    let mut plan = Plan::new(start, std::path::PathBuf::new());
    plan.depth = request.depth;
    plan.types = types_of(&request.kinds, &request.extensions);
    plan.allowed = allowed;
    plan.max_files = usize::try_from(request.max_files).unwrap_or(usize::MAX);
    plan.max_total = request.max_total;
    plan.dry_run = true;
    if !(1..=crawl::MAX_DEPTH).contains(&plan.depth) {
        return Err("depth".into());
    }
    if !(1..=crawl::MAX_FILES).contains(&plan.max_files) {
        return Err("max_files".into());
    }
    if plan.max_total == 0 {
        return Err("max_total".into());
    }
    if plan.types.is_empty() {
        return Err("types".into());
    }
    Ok((plan, scope))
}

/// «Check»: reads the site and lists the files it would download, with
/// `grab-progress` as it goes; a new check stops one running.
#[tauri::command]
pub async fn grab_check<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, GrabState>,
    request: GrabRequest,
    handle: tauri::AppHandle<R>,
) -> Result<GrabScan, String> {
    let (plan, scope) = plan_of(&request)?;
    let stop = Stop::new();
    if let Some(old) = state.running.lock().unwrap().replace(stop.clone()) {
        old.request();
    }
    let host = plan.start.host_str().unwrap_or_default().to_owned();
    let start_origin = plan.start.origin().ascii_serialization();
    let (mut files, mut progress, mut robots_closed) =
        (Vec::new(), GrabProgress { pages: 0, files: 0 }, false);
    let report = crawl::crawl(&app.engine, &plan, &scope, &stop, &mut |event| {
        match event {
            Event::Page { .. } => progress.pages += 1,
            Event::File { url } => {
                progress.files += 1;
                files.push(url.to_string());
            }
            Event::Robots {
                origin,
                outcome: RobotsOutcome::DisallowAll(_),
            } if origin == start_origin => robots_closed = true,
            _ => return,
        }
        let _ = handle.emit("grab-progress", progress);
    })
    .await
    .map_err(|e| e.to_string())?;
    state.running.lock().unwrap().take();
    let id = state.next.fetch_add(1, Ordering::SeqCst) + 1;
    *state.last.lock().unwrap() = Some((id, host, files.clone()));
    Ok(GrabScan {
        id,
        files,
        pages: report.pages as u64,
        skipped: report.skipped.values().sum::<usize>() as u64,
        limits: report
            .limits
            .iter()
            .map(|l| {
                match l {
                    crawl::Limit::Files => "files",
                    crawl::Limit::Pages => "pages",
                    crawl::Limit::Total => "total",
                    crawl::Limit::Waiting => "waiting",
                    crawl::Limit::Seen => "seen",
                }
                .to_owned()
            })
            .collect(),
        robots_closed: robots_closed
            || (report.pages == 0
                && !report.stopped
                && report.skipped.contains_key(&crawl::Skip::Robots)),
        stopped: report.stopped,
    })
}

#[tauri::command]
pub fn grab_cancel(state: State<'_, GrabState>) {
    if let Some(stop) = state.running.lock().unwrap().take() {
        stop.request();
    }
}

/// Adds check `id`'s files, but those at `removed`, to the site's own
/// queue, one download at a time, started or paused; how many were added.
#[tauri::command]
pub async fn grab_add<R: tauri::Runtime>(
    app: State<'_, crate::App>,
    state: State<'_, GrabState>,
    id: u64,
    removed: Vec<u64>,
    folder: Option<String>,
    start: bool,
    handle: tauri::AppHandle<R>,
) -> Result<GrabAdded, String> {
    if crate::in_scene(&handle) {
        return Err("a ui-test scene adds nothing".into());
    }
    let (host, files) = match state.last.lock().unwrap().as_ref() {
        Some((last, host, files)) if *last == id => (host.clone(), files.clone()),
        _ => return Err("this check is no longer the last one: check again".into()),
    };
    let kept = keep(&files, &removed);
    let (store, settings, system) = (
        app.store.clone(),
        app.settings.clone(),
        app.system_download_dir.clone(),
    );
    let added = tokio::task::spawn_blocking(move || {
        let given = crate::downloads::chosen_folder(
            folder.as_deref(),
            &crate::downloads::download_dir(&settings, &system),
        )
        .map_err(StoreError::Refused)?;
        add_to_site_queue(&store, &host, &kept, folder.is_none(), &given, start)
    })
    .await
    .map_err(|e| e.to_string())?
    .map_err(|e| e.to_string())?;
    let _ = handle.emit("downloads-changed", ());
    Ok(added)
}

/// Where a grab's files went.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, TS)]
#[ts(export)]
pub struct GrabAdded {
    pub queue: String,
    #[ts(type = "number")]
    pub count: u64,
}

/// `files` without those at `removed` (places out of range are ignored).
fn keep(files: &[String], removed: &[u64]) -> Vec<String> {
    files
        .iter()
        .enumerate()
        .filter(|(i, _)| !removed.contains(&(*i as u64)))
        .map(|(_, f)| f.clone())
        .collect()
}

/// The site's queue name: its host with `-` for each dot (a queue name has
/// letters, digits, `-` and `_` only), within the name's limit.
fn queue_name(host: &str) -> String {
    let name: String = host
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '-' {
                c
            } else {
                '-'
            }
        })
        .take(zajil_core::queue::MAX_NAME_CHARS)
        .collect();
    let name = name.trim_matches('-').to_owned();
    if name.is_empty() {
        "site".to_owned()
    } else {
        name
    }
}

/// Adds `files` to the site's queue in one write (all or none). A new
/// queue runs one download at a time; one the user already has keeps its
/// own settings.
fn add_to_site_queue(
    store: &QueueStore,
    host: &str,
    files: &[String],
    categorize: bool,
    given: &std::path::Path,
    start: bool,
) -> Result<GrabAdded, StoreError> {
    let name = queue_name(host);
    store.update(|queues: &mut QueuesFile| {
        if queues.queue(&name).is_none() {
            queues.queue_or_new(&name)?.max_active = 1;
        }
        for file in files {
            let id = queues.add(&name, file, given)?;
            let item = queues.item_mut(id).expect("just added");
            item.categorize = categorize;
            // One connection each: the site's manners (#211, ADR-0010).
            item.connections = Some(1);
            if !start {
                item.status = Status::Paused;
            }
        }
        Ok(GrabAdded {
            queue: name.clone(),
            count: files.len() as u64,
        })
    })
}

/// Opens «Site grabber», with `url` as its start if one is given.
#[tauri::command]
pub async fn open_grab<R: tauri::Runtime>(
    state: State<'_, GrabState>,
    url: Option<String>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    if crate::in_scene(&handle) {
        return Ok(());
    }
    *state.pending.lock().unwrap() = url;
    crate::windows::open(&handle, LABEL, "Zajil", SIZE).map_err(|e| e.to_string())
}

/// The start link the window opens with, once.
#[tauri::command]
pub fn grab_pending(state: State<'_, GrabState>) -> Option<String> {
    state.pending.lock().unwrap().take()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn request(start: &str) -> GrabRequest {
        GrabRequest {
            start: start.into(),
            depth: crawl::DEFAULT_DEPTH,
            kinds: vec![
                "archives".into(),
                "documents".into(),
                "audio".into(),
                "video".into(),
                "programs".into(),
            ],
            extensions: vec![],
            sites: vec![],
            max_files: crawl::DEFAULT_MAX_FILES as u64,
            max_total: crawl::DEFAULT_MAX_TOTAL,
        }
    }

    /// The window's kinds by default are `zajil grab`'s own types; images
    /// and written extensions come on top, once each.
    #[test]
    fn the_kinds_are_zajil_grabs_types() {
        let defaults = types_of(&request("https://a.test/").kinds, &[]);
        let mut theirs: Vec<String> = crawl::DEFAULT_TYPES
            .iter()
            .map(|t| (*t).to_owned())
            .collect();
        let mut ours = defaults.clone();
        theirs.sort();
        ours.sort();
        assert_eq!(ours, theirs);
        let more = types_of(
            &["images".into()],
            &[" .PSD ".into(), "jpg".into(), "../x".into(), String::new()],
        );
        assert!(more.contains(&"psd".to_owned()) && more.contains(&"png".to_owned()));
        assert_eq!(more.iter().filter(|t| *t == "jpg").count(), 1);
        assert!(!more.iter().any(|t| t.contains('/') || t.contains('.')));
    }

    /// A request is checked with `zajil grab`'s limits, and robots.txt is
    /// never turned off from the window.
    #[test]
    fn a_request_is_checked() {
        let (plan, _) = plan_of(&request("https://a.test/files/")).unwrap();
        assert!(plan.dry_run && !plan.ignore_robots);
        assert_eq!(plan.depth, crawl::DEFAULT_DEPTH);
        let bad = |change: &dyn Fn(&mut GrabRequest)| {
            let mut r = request("https://a.test/");
            change(&mut r);
            plan_of(&r).unwrap_err()
        };
        assert_eq!(bad(&|r| r.start = "ftp://a.test/".into()), "start");
        assert_eq!(bad(&|r| r.start = "https://u:p@a.test/".into()), "start");
        assert_eq!(bad(&|r| r.depth = 0), "depth");
        assert_eq!(bad(&|r| r.depth = crawl::MAX_DEPTH + 1), "depth");
        assert_eq!(
            bad(&|r| r.max_files = crawl::MAX_FILES as u64 + 1),
            "max_files"
        );
        assert_eq!(bad(&|r| r.max_total = 0), "max_total");
        assert_eq!(bad(&|r| (r.kinds.clear(), r.extensions.clear()).0), "types");
        assert_eq!(
            bad(&|r| r.sites = vec!["cdn.a.test/x".into()]),
            "site:cdn.a.test/x"
        );
        let mut sites = request("https://a.test/");
        sites.sites = vec!["cdn.a.test".into(), " ".into(), "files.a.test:8443".into()];
        assert_eq!(plan_of(&sites).unwrap().0.allowed.len(), 2);
    }

    /// Only the files the page kept go in, from the check's own list.
    #[test]
    fn the_page_takes_files_out_by_place_only() {
        let files: Vec<String> = (0..4).map(|i| format!("https://a.test/{i}.zip")).collect();
        assert_eq!(
            keep(&files, &[1, 3, 99]),
            ["https://a.test/0.zip", "https://a.test/2.zip"]
        );
    }

    /// The site's queue: one download at a time when new, the user's own
    /// settings when it was there; all the files or none.
    #[test]
    fn the_files_go_into_the_site_s_queue() {
        assert_eq!(queue_name("files.example.com"), "files-example-com");
        assert_eq!(queue_name("[::1]"), "1");
        let data = tempfile::tempdir().unwrap();
        let store = QueueStore::new(data.path());
        let files: Vec<String> = (0..3).map(|i| format!("https://a.test/{i}.zip")).collect();
        let added = add_to_site_queue(&store, "a.test", &files, true, data.path(), false).unwrap();
        assert_eq!((added.queue.as_str(), added.count), ("a-test", 3));
        let queues = store.load().unwrap();
        let queue = queues.queue("a-test").unwrap();
        assert_eq!(queue.max_active, 1);
        assert!(
            queue
                .items
                .iter()
                .all(|i| i.status == Status::Paused && i.categorize && i.connections == Some(1))
        );
        store
            .update(|q| {
                q.queue_mut("a-test").unwrap().max_active = 2;
                Ok(())
            })
            .unwrap();
        add_to_site_queue(&store, "a.test", &files[..1], false, data.path(), true).unwrap();
        let queues = store.load().unwrap();
        let queue = queues.queue("a-test").unwrap();
        assert_eq!((queue.max_active, queue.items.len()), (2, 4));
        assert_eq!(queue.items[3].status, Status::Waiting);
        assert_eq!(queue.items[3].connections, Some(1));
        let bad = vec![
            "https://a.test/x.zip".to_owned(),
            "https://u:p@a.test/y.zip".to_owned(),
        ];
        assert!(add_to_site_queue(&store, "a.test", &bad, true, data.path(), true).is_err());
        assert_eq!(
            store.load().unwrap().queue("a-test").unwrap().items.len(),
            4
        );
    }

    #[test]
    fn the_grab_window_has_only_its_own_permissions() {
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
}
