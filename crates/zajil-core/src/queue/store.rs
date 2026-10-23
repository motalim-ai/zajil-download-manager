//! The queues file (ADR-0007): `queues.json` in the data folder.
//!
//! The same rules as the state file (ADR-0004, `docs/QUEUES-FILE.md`):
//! - a `schema_version`, and a SHA-256 `checksum` over the canonical form;
//! - strict reading: a damaged, oversized, linked or unknown file is
//!   refused, never guessed at, and never overwritten;
//! - safe writes (a temporary file, `fsync`, a rename, then the folder
//!   synced), readable by the user only, because a URL may hold a token
//!   (AGENTS.md §10.3);
//! - every change goes through [`QueueStore::update`], which holds
//!   `queues.lock` while it reads, changes and writes, so two `zajil`
//!   processes never write at the same time and no change is lost.
//!
//! A download's progress is not here: it stays in its own `.zajil-state`
//! file (ADR-0004), which an item only points to.

use std::fs::{File, OpenOptions, TryLockError};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use super::schedule::Schedule;
use crate::state::{checksum_of, now_rfc3339, write_safely};

/// Value of the `format` field.
pub const FORMAT: &str = "zajil-queues";
/// The schema this version of Zajil reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// The queue an item goes to when none is named.
pub const DEFAULT_QUEUE: &str = "main";
/// Downloads of one queue that run at once, unless the user sets another
/// number.
pub const DEFAULT_MAX_ACTIVE: u32 = 3;
/// The most downloads one queue may run at once.
pub const MAX_ACTIVE: u32 = 32;
/// The longest queue name, in characters.
pub const MAX_NAME_CHARS: usize = 64;

const FILE_NAME: &str = "queues.json";
const LOCK_NAME: &str = "queues.lock";
const RUNNER_LOCK_NAME: &str = "runner.lock";
/// Thousands of items fit in far less; anything above is refused.
const MAX_FILE_BYTES: u64 = 16 * 1024 * 1024;

/// Everything in `queues.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct QueuesFile {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`SCHEMA_VERSION`] when written by this version.
    pub schema_version: u32,
    /// The id the next item gets. Ids are never reused.
    pub next_id: u64,
    /// Settings for the whole program.
    #[serde(default)]
    pub settings: Settings,
    pub queues: Vec<Queue>,
    /// RFC 3339 time in UTC.
    pub updated_at: String,
    /// `sha256:` and the hex digest of the file with this field empty.
    pub checksum: String,
}

/// Settings for the whole program.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Settings {
    /// Bytes per second for all downloads together; `None`: no limit.
    pub speed_limit: Option<u64>,
}

/// A named list of downloads, run in order.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Queue {
    pub name: String,
    /// Downloads of this queue that run at once, from 1 to [`MAX_ACTIVE`].
    pub max_active: u32,
    /// When it may run; `None`: always.
    #[serde(default)]
    pub schedule: Option<Schedule>,
    pub items: Vec<Item>,
}

/// One download in a queue.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Item {
    pub id: u64,
    /// The URL as given, without a user name or password (those are never
    /// saved, ADR-0004 rule 7). It may hold a token (`?sig=…`); it is
    /// never printed with it.
    pub url: String,
    /// The folder to save into (absolute).
    pub dir: PathBuf,
    pub status: Status,
    /// The download's `.zajil-state` once it has one: its progress.
    pub state_file: Option<PathBuf>,
    /// The finished file.
    pub file: Option<PathBuf>,
    /// Why it failed or stopped, for the user.
    pub error: Option<String>,
    /// Bytes per second for this download alone; `None`: no limit of its
    /// own (the program's limit still applies).
    #[serde(default)]
    pub limit: Option<u64>,
    /// RFC 3339 time in UTC.
    pub added_at: String,
}

/// Where an item stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Status {
    /// Runs when its queue has room.
    Waiting,
    /// A runner is downloading it.
    Active,
    /// The user paused it; it waits until resumed.
    Paused,
    Done,
    /// It stopped with an error; resuming it puts it back to waiting.
    Failed,
}

/// Why the queues file could not be used or changed.
#[derive(Debug, thiserror::Error)]
pub enum StoreError {
    #[error("could not read or write the queues file: {0}")]
    Io(#[from] io::Error),
    #[error("{} is not a Zajil queues file", .0.display())]
    NotAQueuesFile(PathBuf),
    #[error(
        "{} uses schema version {version}, which this version of Zajil cannot read",
        path.display()
    )]
    UnsupportedVersion { path: PathBuf, version: u64 },
    #[error(
        "the queues file {} is damaged ({reason}); Zajil leaves it as it is, so nothing in it is \
         lost: move it away to start with empty queues",
        path.display()
    )]
    Corrupt { path: PathBuf, reason: String },
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
    #[error("there is no item {0}")]
    NoSuchItem(u64),
    #[error("there is no queue {0:?}")]
    NoSuchQueue(String),
    #[error("invalid queue name {0:?}: use 1 to {MAX_NAME_CHARS} letters, digits, `-` or `_`")]
    InvalidName(String),
    #[error("the number of downloads at once must be from 1 to {MAX_ACTIVE}, not {0}")]
    InvalidMaxActive(u32),
    #[error(
        "this URL has a user name or password; Zajil never saves them, so it cannot be queued \
         (download it with `zajil get` instead)"
    )]
    Credentials,
    #[error(transparent)]
    Url(#[from] crate::Error),
    #[error("another `zajil queue run` is already running")]
    RunnerBusy,
}

impl QueuesFile {
    /// No queues yet but the default one, empty.
    pub fn empty() -> Self {
        QueuesFile {
            format: FORMAT.to_owned(),
            schema_version: SCHEMA_VERSION,
            next_id: 1,
            settings: Settings::default(),
            queues: vec![Queue::new(DEFAULT_QUEUE)],
            updated_at: now_rfc3339(),
            checksum: String::new(),
        }
    }

    pub fn queue(&self, name: &str) -> Option<&Queue> {
        self.queues.iter().find(|q| q.name == name)
    }

    pub fn queue_mut(&mut self, name: &str) -> Option<&mut Queue> {
        self.queues.iter_mut().find(|q| q.name == name)
    }

    /// The queue `name`, created (empty, with the defaults) if needed.
    pub fn queue_or_new(&mut self, name: &str) -> Result<&mut Queue, StoreError> {
        check_name(name)?;
        if self.queue(name).is_none() {
            self.queues.push(Queue::new(name));
        }
        Ok(self.queue_mut(name).expect("just added"))
    }

    pub fn item(&self, id: u64) -> Option<&Item> {
        self.queues
            .iter()
            .flat_map(|q| &q.items)
            .find(|i| i.id == id)
    }

    pub fn item_mut(&mut self, id: u64) -> Option<&mut Item> {
        self.queues
            .iter_mut()
            .flat_map(|q| &mut q.items)
            .find(|i| i.id == id)
    }

    /// Adds `url` to the end of queue `queue` (created if needed), to be
    /// saved in `dir`. Only http and https URLs, and never one with a user
    /// name or password (they would have to be saved). Returns its id.
    pub fn add(&mut self, queue: &str, url: &str, dir: &Path) -> Result<u64, StoreError> {
        let parsed = crate::engine::parse_url(url)?;
        if !parsed.username().is_empty() || parsed.password().is_some() {
            return Err(StoreError::Credentials);
        }
        let id = self.next_id;
        let item = Item {
            id,
            url: url.to_owned(),
            dir: std::path::absolute(dir)?,
            status: Status::Waiting,
            state_file: None,
            file: None,
            error: None,
            limit: None,
            added_at: now_rfc3339(),
        };
        self.queue_or_new(queue)?.items.push(item);
        self.next_id = id.checked_add(1).ok_or_else(|| StoreError::Corrupt {
            path: PathBuf::new(),
            reason: "no ids are left".into(),
        })?;
        Ok(id)
    }

    /// Removes item `id` from its queue and returns it. Its files stay.
    pub fn remove(&mut self, id: u64) -> Result<Item, StoreError> {
        for queue in &mut self.queues {
            if let Some(index) = queue.items.iter().position(|i| i.id == id) {
                return Ok(queue.items.remove(index));
            }
        }
        Err(StoreError::NoSuchItem(id))
    }

    /// Rejects values that cannot be a real queues file even when the
    /// checksum matches: repeated or too large ids, repeated or invalid
    /// queue names, impossible limits.
    fn check_consistency(&self) -> Result<(), String> {
        if self.settings.speed_limit == Some(0) {
            return Err("a speed limit of 0".into());
        }
        let mut names = std::collections::HashSet::new();
        let mut ids = std::collections::HashSet::new();
        for queue in &self.queues {
            check_name(&queue.name).map_err(|e| e.to_string())?;
            if !names.insert(&queue.name) {
                return Err(format!("the queue {:?} appears twice", queue.name));
            }
            if !(1..=MAX_ACTIVE).contains(&queue.max_active) {
                return Err(format!("impossible max_active {}", queue.max_active));
            }
            for item in &queue.items {
                if item.id >= self.next_id || !ids.insert(item.id) {
                    return Err(format!("impossible or repeated id {}", item.id));
                }
                if item.limit == Some(0) {
                    return Err(format!("a speed limit of 0 for item {}", item.id));
                }
            }
        }
        Ok(())
    }
}

impl Queue {
    fn new(name: &str) -> Self {
        Queue {
            name: name.to_owned(),
            max_active: DEFAULT_MAX_ACTIVE,
            schedule: None,
            items: Vec::new(),
        }
    }
}

/// A queue name: 1 to [`MAX_NAME_CHARS`] letters (any script), digits, `-`
/// or `_`. Nothing that could confuse a terminal or a file name.
pub fn check_name(name: &str) -> Result<(), StoreError> {
    let count = name.chars().count();
    let ok = (1..=MAX_NAME_CHARS).contains(&count)
        && name
            .chars()
            .all(|c| c.is_alphanumeric() || c == '-' || c == '_');
    if ok {
        Ok(())
    } else {
        Err(StoreError::InvalidName(name.to_owned()))
    }
}

/// The queues file in one folder, and the locks next to it.
#[derive(Debug, Clone)]
pub struct QueueStore {
    dir: PathBuf,
}

/// Held while a runner runs: only one runner at a time.
#[derive(Debug)]
pub struct RunnerLock {
    _file: File,
}

impl QueueStore {
    /// The store in `dir` (normally `AppPaths::data_dir`). Nothing is
    /// created until the first change.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        QueueStore { dir: dir.into() }
    }

    /// `queues.json`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// Reads and fully checks the queues file; a missing file is
    /// [`QueuesFile::empty`]. No lock is needed: every write replaces the
    /// file in one step, so a reader sees the old file or the new one.
    /// Blocking: async code calls it from a blocking task.
    pub fn load(&self) -> Result<QueuesFile, StoreError> {
        let path = self.path();
        let file = match zajil_platform::fs::open_regular_nofollow(&path, false) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(QueuesFile::empty()),
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(StoreError::NotRegularFile(path));
            }
            Err(err) => return Err(err.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        let corrupt = |reason: String| StoreError::Corrupt {
            path: path.clone(),
            reason,
        };
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(corrupt("the file is too large".into()));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(e.to_string()))?;
        if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
            return Err(StoreError::NotAQueuesFile(path));
        }
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(SCHEMA_VERSION) => {}
            Some(version) => return Err(StoreError::UnsupportedVersion { path, version }),
            None => return Err(corrupt("missing schema_version".into())),
        }
        let Some(stored) = value.get("checksum").and_then(|v| v.as_str()) else {
            return Err(corrupt("missing checksum".into()));
        };
        if stored != checksum_of(&value) {
            return Err(corrupt("its checksum does not match".into()));
        }
        let queues: QueuesFile =
            serde_json::from_value(value).map_err(|e| corrupt(e.to_string()))?;
        queues.check_consistency().map_err(corrupt)?;
        Ok(queues)
    }

    /// Changes the queues file with `change`, holding `queues.lock` from
    /// the read to the end of the write: changes from several processes
    /// follow each other and none is lost. A damaged file is never
    /// overwritten. Blocking: it waits for the lock.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut QueuesFile) -> Result<T, StoreError>,
    ) -> Result<T, StoreError> {
        let _lock = self.lock(LOCK_NAME, true)?;
        let mut queues = self.load()?;
        let result = change(&mut queues)?;
        queues.format = FORMAT.to_owned();
        queues.schema_version = SCHEMA_VERSION;
        queues.updated_at = now_rfc3339();
        queues.checksum = String::new();
        queues.checksum = checksum_of(&serde_json::to_value(&queues).expect("serializes"));
        let mut json = serde_json::to_vec_pretty(&queues).map_err(io::Error::other)?;
        json.push(b'\n');
        write_safely(&self.path(), &json)?;
        Ok(result)
    }

    /// Takes the runner lock, or fails with [`StoreError::RunnerBusy`] if
    /// another runner holds it. Released when dropped (or when the process
    /// ends, even if it is killed).
    pub fn lock_runner(&self) -> Result<RunnerLock, StoreError> {
        Ok(RunnerLock {
            _file: self.lock(RUNNER_LOCK_NAME, false)?,
        })
    }

    /// Opens (creating if needed) the lock file `name` and locks it,
    /// waiting for it if `wait`. Never through a link (AGENTS.md §10.1):
    /// an existing file is opened with `open_regular_nofollow`, a missing
    /// one created with `create_new`.
    fn lock(&self, name: &str, wait: bool) -> Result<File, StoreError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(name);
        let file = loop {
            match zajil_platform::fs::open_regular_nofollow(&path, true) {
                Ok(file) => break file,
                Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                    return Err(StoreError::NotRegularFile(path));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                // Another process created it first: open that one.
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
        };
        if wait {
            file.lock()?;
        } else {
            match file.try_lock() {
                Ok(()) => {}
                Err(TryLockError::WouldBlock) => return Err(StoreError::RunnerBusy),
                Err(TryLockError::Error(err)) => return Err(err.into()),
            }
        }
        Ok(file)
    }
}

/// `url` as it may be shown: without a user name, password, query or
/// fragment, which may hold secrets (AGENTS.md §10.3).
pub fn shown_url(url: &str) -> String {
    match reqwest::Url::parse(url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.set_query(None);
            url.set_fragment(None);
            url.to_string()
        }
        Err(_) => "(invalid URL)".to_owned(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn queue_names_are_plain() {
        for good in ["main", "ليلي", "night_2", "a-b"] {
            check_name(good).unwrap();
        }
        let long = "a".repeat(MAX_NAME_CHARS + 1);
        for bad in ["", "a b", "a/b", "..", "a\u{202e}b", "a\nb", long.as_str()] {
            assert!(check_name(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn shown_urls_hold_no_secrets() {
        assert_eq!(
            shown_url("https://u:p@h/f.bin?sig=SECRET#x"),
            "https://h/f.bin"
        );
        assert_eq!(shown_url("not a url"), "(invalid URL)");
    }

    #[test]
    fn ids_are_never_reused() {
        let mut queues = QueuesFile::empty();
        let dir = std::env::temp_dir();
        let a = queues.add("main", "http://h/a", &dir).unwrap();
        queues.remove(a).unwrap();
        let b = queues.add("main", "http://h/b", &dir).unwrap();
        assert!(b > a);
        assert!(matches!(
            queues.add("main", "http://u:p@h/c", &dir),
            Err(StoreError::Credentials)
        ));
        assert!(queues.add("main", "ftp://h/c", &dir).is_err());
    }
}
