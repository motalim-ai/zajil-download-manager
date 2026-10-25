//! Export and import of queues (M4, owner's decision on #36), in a file
//! of its own format (`docs/QUEUES-FILE.md`, "Export file").
//!
//! - What goes in: the program's settings, and each queue with its
//!   settings and its downloads (URL, folder, status, limit, whether it
//!   goes by category). Not the progress, the state files, the finished
//!   files or errors: those belong to this computer.
//! - Never a user name or password: queues cannot hold them, and an
//!   export that somehow met one would refuse it. A token that is part of
//!   a URL (`?sig=…`) is kept, under the two conditions of AGENTS.md
//!   §10.3: the file is readable by its owner only, and the token is
//!   never printed.
//! - An export file is untrusted input when it is read: a size limit,
//!   strict parsing (unknown fields refused), a checksum, and every value
//!   checked before anything changes. A damaged or hostile file is
//!   refused with a clear error, never half applied.
//! - Folders in the file are used only when the user asks
//!   ([`Folders::Keep`]): a file from someone else could otherwise send
//!   downloads into folders such as a startup folder.

use std::collections::HashMap;
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::schedule::Schedule;
use super::store::{
    DEFAULT_MAX_ACTIVE, Item, MAX_ACTIVE, Queue, QueuesFile, Settings, Status, StoreError,
    check_name,
};
use crate::category::{has_hidden, is_unsafe_in_url, printable};
use crate::state::{checksum_of, now_rfc3339};

/// Value of the `format` field.
pub const EXPORT_FORMAT: &str = "zajil-queues-export";
/// The schema this version of Zajil reads and writes.
pub const EXPORT_SCHEMA_VERSION: u32 = 1;
/// Larger files are refused before they are parsed.
pub const MAX_EXPORT_BYTES: u64 = 16 * 1024 * 1024;
/// The whole export file.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportFile {
    pub format: String,
    pub schema_version: u32,
    #[serde(default)]
    pub settings: Settings,
    pub queues: Vec<ExportQueue>,
    /// RFC 3339 time in UTC.
    pub exported_at: String,
    /// `sha256:` and the hex digest of the file with this field empty.
    pub checksum: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportQueue {
    pub name: String,
    pub max_active: u32,
    #[serde(default)]
    pub schedule: Option<Schedule>,
    pub items: Vec<ExportItem>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ExportItem {
    pub url: String,
    pub dir: PathBuf,
    pub status: ExportStatus,
    #[serde(default)]
    pub limit: Option<u64>,
    #[serde(default)]
    pub categorize: bool,
    pub added_at: String,
}

/// Where an exported download stands. A running or failed download is
/// exported as waiting: on the other side it runs again.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExportStatus {
    Waiting,
    Paused,
    Done,
}

/// Which folders imported downloads go to.
#[derive(Debug, Clone)]
pub enum Folders {
    /// The folders written in the file (the user asked for them).
    Keep,
    /// This folder, and each download's category folder (M4).
    Into(PathBuf),
}

/// What an import added.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ImportSummary {
    pub queues_created: usize,
    /// Queues that existed with settings or downloads of their own: they
    /// kept their settings, and the file's were not used.
    pub kept_settings: Vec<String>,
    pub items_added: usize,
    pub speed_limit_set: bool,
}

/// Why an export could not be written or read.
#[derive(Debug, thiserror::Error)]
pub enum TransferError {
    #[error("could not read or write the export file: {0}")]
    Io(#[from] io::Error),
    #[error("{} already exists; Zajil never writes over a file: choose another name", .0.display())]
    Exists(PathBuf),
    #[error("{} is not a Zajil queues export", .0.display())]
    NotAnExport(PathBuf),
    #[error(
        "{} uses export version {version}, which this version of Zajil cannot read",
        path.display()
    )]
    UnsupportedVersion { path: PathBuf, version: u64 },
    #[error("refusing to import {}: {reason}; nothing was changed", path.display())]
    Refused { path: PathBuf, reason: String },
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
    #[error(transparent)]
    Store(#[from] StoreError),
}

impl QueuesFile {
    /// Everything that belongs in an export (see the module docs).
    pub fn export(&self) -> ExportFile {
        let queues = self
            .queues
            .iter()
            .map(|q| ExportQueue {
                name: q.name.clone(),
                max_active: q.max_active,
                schedule: q.schedule.clone(),
                items: q
                    .items
                    .iter()
                    .map(|i| ExportItem {
                        url: without_credentials(&i.url),
                        dir: i.dir.clone(),
                        status: match i.status {
                            Status::Paused => ExportStatus::Paused,
                            Status::Done => ExportStatus::Done,
                            Status::Waiting | Status::Active | Status::Failed => {
                                ExportStatus::Waiting
                            }
                        },
                        limit: i.limit,
                        categorize: i.categorize,
                        added_at: i.added_at.clone(),
                    })
                    .collect(),
            })
            .collect();
        ExportFile {
            format: EXPORT_FORMAT.to_owned(),
            schema_version: EXPORT_SCHEMA_VERSION,
            settings: self.settings.clone(),
            queues,
            exported_at: now_rfc3339(),
            checksum: String::new(),
        }
    }

    /// Adds what `export` holds (checked already by [`read_export`]):
    /// queues that do not exist are created with the file's settings; the
    /// downloads of a queue that exists go to its end, and its settings
    /// stay. The file's program-wide speed limit is used only when there
    /// is none yet.
    pub fn import(
        &mut self,
        export: &ExportFile,
        folders: &Folders,
    ) -> Result<ImportSummary, StoreError> {
        // The file's folders, only when the user asked for them, and
        // checked before anything changes (review of #41: they were always
        // checked, so an export from another system was refused even when
        // its folders were not used).
        if matches!(folders, Folders::Keep) {
            for q in &export.queues {
                for (n, i) in q.items.iter().enumerate() {
                    check_folder_from_file(&i.dir).map_err(|why| {
                        StoreError::Refused(format!(
                            "download {} of queue {:?} {why}",
                            n + 1,
                            q.name
                        ))
                    })?;
                }
            }
        }
        let mut summary = ImportSummary::default();
        if self.settings.speed_limit.is_none() && export.settings.speed_limit.is_some() {
            self.settings.speed_limit = export.settings.speed_limit;
            summary.speed_limit_set = true;
        }
        // Each queue by name, found at once: a search per queue and per
        // download made a file of many queues take minutes, holding the
        // queues lock (review of #41, round 3).
        let mut index: HashMap<String, usize> = self
            .queues
            .iter()
            .enumerate()
            .map(|(n, q)| (q.name.clone(), n))
            .collect();
        for eq in &export.queues {
            let at = index.get(&eq.name).copied();
            match at.map(|n| &mut self.queues[n]) {
                None => {
                    index.insert(eq.name.clone(), self.queues.len());
                    self.queues.push(Queue {
                        name: eq.name.clone(),
                        max_active: eq.max_active,
                        schedule: eq.schedule.clone(),
                        items: Vec::new(),
                    });
                    summary.queues_created += 1;
                }
                // A queue nobody has touched (empty, with the defaults, like
                // the `main` of a new data folder) takes the file's settings
                // (review of #41: `main` lost them, so the round trip into a
                // new data folder was not exact).
                Some(q)
                    if q.items.is_empty()
                        && q.max_active == DEFAULT_MAX_ACTIVE
                        && q.schedule.is_none() =>
                {
                    q.max_active = eq.max_active;
                    q.schedule = eq.schedule.clone();
                }
                Some(q) => {
                    if q.max_active != eq.max_active || q.schedule != eq.schedule {
                        summary.kept_settings.push(q.name.clone());
                    }
                }
            }
            for ei in &eq.items {
                let id = self.next_id;
                self.next_id = id.checked_add(1).ok_or_else(|| StoreError::Corrupt {
                    path: PathBuf::new(),
                    reason: "no ids are left".into(),
                })?;
                let (dir, categorize) = match folders {
                    Folders::Keep => (ei.dir.clone(), ei.categorize),
                    Folders::Into(dir) => (std::path::absolute(dir)?, true),
                };
                let item = Item {
                    id,
                    url: ei.url.clone(),
                    dir,
                    status: match ei.status {
                        ExportStatus::Waiting => Status::Waiting,
                        ExportStatus::Paused => Status::Paused,
                        ExportStatus::Done => Status::Done,
                    },
                    state_file: None,
                    file: None,
                    error: None,
                    limit: ei.limit,
                    categorize,
                    needs_ok: false,
                    from_page: false,
                    ok_given: false,
                    private: false,
                    added_at: ei.added_at.clone(),
                    // An export says nothing about a browser.
                    browser: None,
                };
                self.queues[index[&eq.name]].items.push(item);
                summary.items_added += 1;
            }
        }
        Ok(summary)
    }
}

/// Writes `export` to `path`, which must not exist yet, readable by the
/// user only (a URL may hold a token). Blocking.
///
/// The data goes to a new temporary file of its own name next to `path`
/// (created only if nothing is there), is synced, then takes `path` with a
/// rename that never replaces: nothing that exists is removed or written
/// over, not a `<file>.tmp` of the user's, and not a file that appears at
/// `path` meanwhile (review of #41).
pub fn write_export(path: &Path, export: &ExportFile) -> Result<(), TransferError> {
    use std::io::Write as _;
    if std::fs::symlink_metadata(path).is_ok() {
        return Err(TransferError::Exists(path.to_owned()));
    }
    let mut export = export.clone();
    export.checksum = String::new();
    export.checksum = checksum_of(&serde_json::to_value(&export).map_err(io::Error::other)?);
    let mut json = serde_json::to_vec_pretty(&export).map_err(io::Error::other)?;
    json.push(b'\n');
    if path.file_name().is_none() {
        return Err(io::Error::new(io::ErrorKind::InvalidInput, "no file name").into());
    }
    let folder = match path.parent() {
        Some(p) if !p.as_os_str().is_empty() => p.to_path_buf(),
        _ => PathBuf::from("."),
    };
    let (tmp, mut file) = (0u32..)
        .take(100)
        .find_map(|n| {
            // Short and fixed, so a long export name still fits (review of
            // #41, round 2).
            let tmp = folder.join(format!(".zajil-export-{}-{n}.tmp", std::process::id()));
            match zajil_platform::fs::create_private(&tmp) {
                Ok(file) => Some(Ok((tmp, file))),
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => None,
                Err(err) => Some(Err(err)),
            }
        })
        .unwrap_or_else(|| Err(io::Error::other("no free temporary name")))?;
    let written = file
        .write_all(&json)
        .and_then(|()| file.sync_all())
        .and_then(|()| {
            drop(file);
            zajil_platform::fs::rename_noreplace(&tmp, path)
        });
    if let Err(err) = written {
        let _ = std::fs::remove_file(&tmp);
        return Err(if err.kind() == io::ErrorKind::AlreadyExists {
            TransferError::Exists(path.to_owned())
        } else {
            err.into()
        });
    }
    zajil_platform::fs::sync_dir(&folder)?;
    Ok(())
}

/// Reads and fully checks an export file. Blocking.
pub fn read_export(path: &Path) -> Result<ExportFile, TransferError> {
    let file = match zajil_platform::fs::open_regular_nofollow(path, false) {
        Ok(file) => file,
        Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
            return Err(TransferError::NotRegularFile(path.to_owned()));
        }
        Err(err) => return Err(err.into()),
    };
    let refused = |reason: String| TransferError::Refused {
        path: path.to_owned(),
        reason,
    };
    let mut bytes = Vec::new();
    file.take(MAX_EXPORT_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_EXPORT_BYTES {
        return Err(refused(format!(
            "it is larger than {} MiB",
            MAX_EXPORT_BYTES / 1024 / 1024
        )));
    }
    // serde_json limits nesting (128 levels), so a deeply nested file is
    // an error here, not a stack overflow.
    let value: serde_json::Value = serde_json::from_slice(&bytes)
        .map_err(|e| refused(format!("it is not valid JSON ({e})")))?;
    if value.get("format").and_then(|v| v.as_str()) != Some(EXPORT_FORMAT) {
        return Err(TransferError::NotAnExport(path.to_owned()));
    }
    match value.get("schema_version").and_then(|v| v.as_u64()) {
        Some(v) if v == u64::from(EXPORT_SCHEMA_VERSION) => {}
        Some(version) => {
            return Err(TransferError::UnsupportedVersion {
                path: path.to_owned(),
                version,
            });
        }
        None => return Err(refused("it has no schema_version".into())),
    }
    let Some(stored) = value.get("checksum").and_then(|v| v.as_str()) else {
        return Err(refused("it has no checksum".into()));
    };
    if stored != checksum_of(&value) {
        return Err(refused(
            "its checksum does not match (it was changed or damaged)".into(),
        ));
    }
    let export: ExportFile =
        serde_json::from_value(value).map_err(|e| refused(printable(&e.to_string())))?;
    check(&export).map_err(refused)?;
    Ok(export)
}

/// Every value of an export that passed its checksum, before anything is
/// imported.
fn check(export: &ExportFile) -> Result<(), String> {
    if export.settings.speed_limit == Some(0) {
        return Err("a speed limit of 0".into());
    }
    if export.exported_at.len() > 64 || export.exported_at.parse::<jiff::Timestamp>().is_err() {
        return Err("its exported_at is not an RFC 3339 time".into());
    }
    let mut names = std::collections::HashSet::new();
    for q in &export.queues {
        check_name(&q.name).map_err(|e| e.to_string())?;
        if !names.insert(&q.name) {
            return Err(format!("the queue {:?} appears twice", q.name));
        }
        // A schedule as `Schedule::new` makes it: not empty, each day once
        // and in order (review of #41: a million "mon" made the queues file
        // too large to read back).
        if let Some(schedule) = &q.schedule {
            let valid = Schedule::new(schedule.start, schedule.stop, schedule.days.clone())
                .is_ok_and(|made| made == *schedule);
            if !valid {
                return Err(format!("queue {:?} has an impossible schedule", q.name));
            }
        }
        if !(1..=MAX_ACTIVE).contains(&q.max_active) {
            return Err(format!(
                "queue {:?} runs {} downloads at once (1 to {MAX_ACTIVE} allowed)",
                q.name, q.max_active
            ));
        }
        for (n, i) in q.items.iter().enumerate() {
            let at = format!("download {} of queue {:?}", n + 1, q.name);
            let url = crate::engine::parse_url(&i.url).map_err(|e| format!("{at}: {e}"))?;
            if !url.username().is_empty() || url.password().is_some() {
                return Err(format!(
                    "{at} has a user name or password in its URL, which Zajil never saves"
                ));
            }
            if i.limit == Some(0) {
                return Err(format!("{at} has a speed limit of 0"));
            }
            // The same rule as `QueuesFile::add`, so Zajil always imports
            // its own exports (review of #41, round 4: an emoji with
            // U+FE0F, or a Persian ZWNJ, in a URL was refused).
            if i.url.chars().any(is_unsafe_in_url) {
                return Err(format!(
                    "{at} has control characters or direction marks in its URL"
                ));
            }
            if i.added_at.len() > 64 || i.added_at.parse::<jiff::Timestamp>().is_err() {
                return Err(format!("{at} has an added_at that is not an RFC 3339 time"));
            }
        }
    }
    Ok(())
}

/// `url` without a user name or password (queues never hold them; this
/// keeps an export clean even if one did).
fn without_credentials(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut parsed) if !parsed.username().is_empty() || parsed.password().is_some() => {
            let _ = parsed.set_username("");
            let _ = parsed.set_password(None);
            parsed.to_string()
        }
        _ => url.to_owned(),
    }
}

/// A folder written in an export, used with `--keep-folders`: absolute,
/// without `..`, and on Windows a plain drive path. A network path
/// (`\\server\share`) or a device path (`\\?\`, `\\.\`) would make
/// Windows contact that server or device (review of #41).
fn check_folder_from_file(dir: &Path) -> Result<(), String> {
    use std::path::{Component, Prefix};
    let plain = dir.components().all(|c| match c {
        Component::Prefix(p) => matches!(p.kind(), Prefix::Disk(_)),
        Component::ParentDir => false,
        _ => true,
    });
    if !(dir.is_absolute() && plain) {
        return Err("has a folder that is not a plain absolute path (relative, `..`, or a network or device path)".into());
    }
    // Shown in messages and used as a folder name: no control or invisible
    // characters, which could drive a terminal or disguise the name
    // (review of #41, round 2).
    match dir.to_str() {
        Some(text) if !has_hidden(text) => Ok(()),
        _ => Err(
            "has a folder with control or invisible characters, or that is not valid text".into(),
        ),
    }
}
