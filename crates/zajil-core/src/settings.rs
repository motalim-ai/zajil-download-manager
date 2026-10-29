//! `settings.json`: the desktop interface's settings (M4, plan on #6),
//! by the same rules as `categories.json` and `queues.json` (ADR-0007):
//! a schema version, strict reading, a checksum, owner-only safe writes,
//! a lock from read to write, and never written over when damaged.
//!
//! The program-wide speed limit is not here: it stays in `queues.json`
//! (`settings.speed_limit`), where the runner and `zajil` read it. The
//! engine itself never reads this file; the interface hands the values
//! over (`Engine::with_connections`, `Runner::connections`).

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::category::check_folder;
use crate::state::{checksum_of, now_rfc3339, write_safely};

/// The value of `format`.
pub const FORMAT: &str = "zajil-settings";
/// The only schema this version reads and writes.
pub const SCHEMA_VERSION: u32 = 1;

const FILE_NAME: &str = "settings.json";
const LOCK_NAME: &str = "settings.lock";
const MAX_FILE_BYTES: u64 = 64 * 1024;

/// The interface's language.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Language {
    #[default]
    Ar,
    En,
}

/// Light, dark, or as the system is.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Theme {
    #[default]
    System,
    Light,
    Dark,
}

/// Western digits (123, the default by the owner's choice of design) or
/// Eastern Arabic digits (١٢٣).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, Default)]
#[serde(rename_all = "snake_case")]
pub enum Digits {
    #[default]
    Latn,
    Arab,
}

/// Everything in `settings.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SettingsFile {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`SCHEMA_VERSION`] when written by this version.
    pub schema_version: u32,
    pub language: Language,
    pub theme: Theme,
    pub digits: Digits,
    /// Where new downloads go; `None`: the system's downloads folder.
    pub download_dir: Option<PathBuf>,
    /// Connections for each new download, from 1 to
    /// [`crate::MAX_CONNECTIONS`].
    pub connections: u32,
    /// A system notification when a download finishes or fails (M4).
    /// Absent in files written before it existed, which counts as on.
    #[serde(default = "on")]
    pub notifications: bool,
    /// Clipboard monitoring (M5): a download link the user copies is
    /// offered in the add dialog. Off by default, and when absent.
    #[serde(default)]
    pub clipboard: bool,
    /// Look for a newer Zajil on GitHub at start (#124): only asks, never
    /// downloads. Absent in files written before it existed, which counts
    /// as on.
    #[serde(default = "on")]
    pub update_check: bool,
    /// The most connections at once to each listed site, shared by all its
    /// downloads ([`crate::sites`], owner's decision of 2026-10-26). Empty
    /// by default, and when absent: no site has a limit. Not written while
    /// empty, so a file without limits stays as it was.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub site_connections: Vec<SiteConnections>,
    /// The proxy every download goes through (the owner's request: a
    /// «Proxy» section as in other download managers), such as
    /// `http://proxy.example:8080`: http or https, a host and a port, no
    /// user name or password (never saved, as for downloads). `None`, and
    /// when absent: none. Not written while unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub proxy: Option<String>,
    /// How many video downloads run at once, 1 to [`MAX_VIDEOS_AT_ONCE`]
    /// (the owner's request, 2026-10-29: a playlist ran two at a time).
    /// `None`, and when absent: [`DEFAULT_VIDEOS_AT_ONCE`]. Not written
    /// while unset.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub videos_at_once: Option<u32>,
    /// RFC 3339 time in UTC.
    pub updated_at: String,
    /// `sha256:` and the hex digest of the file with this field empty.
    pub checksum: String,
}

/// Why the settings file could not be used or changed.
#[derive(Debug, thiserror::Error)]
pub enum SettingsError {
    #[error("could not read or write the settings file: {0}")]
    Io(#[from] io::Error),
    #[error("{} is not a Zajil settings file", .0.display())]
    NotASettingsFile(PathBuf),
    #[error(
        "{} uses schema version {version}, which this version of Zajil cannot read",
        path.display()
    )]
    UnsupportedVersion { path: PathBuf, version: u64 },
    #[error(
        "the settings file {} is damaged ({reason}); Zajil leaves it as it is, so nothing in it \
         is lost: move it away to start again with the default settings",
        path.display()
    )]
    Corrupt { path: PathBuf, reason: String },
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
    #[error("{0}")]
    Invalid(String),
}

fn on() -> bool {
    true
}

impl Default for SettingsFile {
    fn default() -> Self {
        SettingsFile {
            format: FORMAT.to_owned(),
            schema_version: SCHEMA_VERSION,
            language: Language::default(),
            theme: Theme::default(),
            digits: Digits::default(),
            download_dir: None,
            connections: crate::DEFAULT_CONNECTIONS as u32,
            notifications: true,
            clipboard: false,
            update_check: true,
            site_connections: Vec::new(),
            proxy: None,
            videos_at_once: None,
            updated_at: now_rfc3339(),
            checksum: String::new(),
        }
    }
}

/// Video downloads at once by default: each YouTube one takes about 12
/// connections (8 for the picture, 4 for the sound).
pub const DEFAULT_VIDEOS_AT_ONCE: u32 = 3;
/// The most video downloads at once.
pub const MAX_VIDEOS_AT_ONCE: u32 = 8;

impl SettingsFile {
    /// How many video downloads run at once.
    pub fn videos_at_once(&self) -> u32 {
        self.videos_at_once.unwrap_or(DEFAULT_VIDEOS_AT_ONCE)
    }

    /// Every value in its range: a file that breaks this is refused when
    /// read, and never written.
    pub fn check(&self) -> Result<(), String> {
        if !(1..=crate::MAX_CONNECTIONS as u32).contains(&self.connections) {
            return Err(format!(
                "connections must be from 1 to {}, not {}",
                crate::MAX_CONNECTIONS,
                self.connections
            ));
        }
        if let Some(dir) = &self.download_dir {
            check_folder(dir).map_err(|e| e.to_string())?;
        }
        if self.site_connections.len() > crate::sites::MAX_SITES {
            return Err(format!(
                "at most {} sites may have a connection limit",
                crate::sites::MAX_SITES
            ));
        }
        if let Some(proxy) = &self.proxy {
            check_proxy(proxy)?;
        }
        if let Some(n) = self.videos_at_once
            && !(1..=MAX_VIDEOS_AT_ONCE).contains(&n)
        {
            return Err(format!(
                "videos at once must be from 1 to {MAX_VIDEOS_AT_ONCE}, not {n}"
            ));
        }
        let mut seen = Vec::new();
        for entry in &self.site_connections {
            let site = entry.site.as_str();
            let host = crate::Url::parse(&format!("http://{site}/"))
                .ok()
                .filter(|u| u.port().is_none() && u.path() == "/")
                .and_then(|u| u.host_str().map(str::to_owned));
            // Only what a host can be, so nothing that would never match
            // (`*.example.com`) nor the same site twice (`example.com.`;
            // review of #166).
            let plain = site
                .bytes()
                .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b"-.:[]".contains(&b))
                && !site.starts_with('.')
                && !site.ends_with('.');
            if host.as_deref() != Some(site) || !plain {
                return Err(format!(
                    "{site:?} is not a site name: write it in small letters, without http:// or a path, such as example.com (a name in other letters in its xn-- form)"
                ));
            }
            if !(1..=crate::MAX_CONNECTIONS as u32).contains(&entry.connections) {
                return Err(format!(
                    "connections for {site} must be from 1 to {}, not {}",
                    crate::MAX_CONNECTIONS,
                    entry.connections
                ));
            }
            if seen.contains(&site) {
                return Err(format!("{site} is listed twice"));
            }
            seen.push(site);
        }
        Ok(())
    }
}

/// A proxy as [`SettingsFile::proxy`] takes it: `http://` or `https://`, a
/// host and a port, nothing else (no user name or password, no path).
pub fn check_proxy(proxy: &str) -> Result<(), String> {
    let wrong = || {
        format!(
            "{:?} is not a proxy: write it as http://host:port",
            crate::category::printable(proxy)
        )
    };
    let url = crate::Url::parse(proxy).map_err(|_| wrong())?;
    if !matches!(url.scheme(), "http" | "https")
        || url.host_str().is_none()
        || url.port_or_known_default().is_none()
        || !matches!(url.path(), "" | "/")
        || url.query().is_some()
        || url.fragment().is_some()
    {
        return Err(wrong());
    }
    if !url.username().is_empty() || url.password().is_some() {
        return Err(
            "a proxy with a user name or password is not supported: Zajil never saves passwords"
                .into(),
        );
    }
    Ok(())
}

/// One site's connection limit in [`SettingsFile::site_connections`].
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SiteConnections {
    /// The site, such as `example.com` (its subdomains too).
    pub site: String,
    /// The most connections at once to it, from 1 to
    /// [`crate::MAX_CONNECTIONS`].
    pub connections: u32,
}

/// The settings file in one folder, and its lock.
#[derive(Debug, Clone)]
pub struct SettingsStore {
    dir: PathBuf,
}

impl SettingsStore {
    /// The store in `dir` (normally `AppPaths::data_dir`). Nothing is
    /// created until the first change.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        SettingsStore { dir: dir.into() }
    }

    /// `settings.json`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// Reads and fully checks the file; a missing file is the defaults.
    /// Blocking.
    pub fn load(&self) -> Result<SettingsFile, SettingsError> {
        let path = self.path();
        let file = match zajil_platform::fs::open_regular_nofollow(&path, false) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(SettingsFile::default());
            }
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(SettingsError::NotRegularFile(path));
            }
            Err(err) => return Err(err.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        let corrupt = |reason: String| SettingsError::Corrupt {
            path: path.clone(),
            reason,
        };
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(corrupt("the file is too large".into()));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(e.to_string()))?;
        if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
            return Err(SettingsError::NotASettingsFile(path));
        }
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(SCHEMA_VERSION) => {}
            Some(version) => return Err(SettingsError::UnsupportedVersion { path, version }),
            None => return Err(corrupt("missing schema_version".into())),
        }
        let Some(stored) = value.get("checksum").and_then(|v| v.as_str()) else {
            return Err(corrupt("missing checksum".into()));
        };
        if stored != checksum_of(&value) {
            return Err(corrupt("its checksum does not match".into()));
        }
        let file: SettingsFile =
            serde_json::from_value(value).map_err(|e| corrupt(e.to_string()))?;
        file.check().map_err(corrupt)?;
        Ok(file)
    }

    /// Changes the file with `change`, holding `settings.lock` from the
    /// read to the end of the write. A damaged file is never overwritten,
    /// and nothing is written if the result is not valid. Blocking.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut SettingsFile) -> Result<T, SettingsError>,
    ) -> Result<T, SettingsError> {
        let _lock = self.lock()?;
        let mut file = self.load()?;
        let result = change(&mut file)?;
        file.check().map_err(SettingsError::Invalid)?;
        file.format = FORMAT.to_owned();
        file.schema_version = SCHEMA_VERSION;
        file.updated_at = now_rfc3339();
        file.checksum = String::new();
        file.checksum = checksum_of(&serde_json::to_value(&file).map_err(io::Error::other)?);
        let mut json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        json.push(b'\n');
        write_safely(&self.path(), &json)?;
        Ok(result)
    }

    /// Opens (creating if needed) `settings.lock` and waits for its lock,
    /// never through a link (AGENTS.md §10.1).
    fn lock(&self) -> Result<File, SettingsError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(LOCK_NAME);
        let file = loop {
            match zajil_platform::fs::open_regular_nofollow(&path, true) {
                Ok(file) => break file,
                Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                    return Err(SettingsError::NotRegularFile(path));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
        };
        file.lock()?;
        Ok(file)
    }
}

/// `dir` as a download folder the settings accept: absolute, without
/// `..`, in valid text without control or invisible characters.
pub fn check_download_dir(dir: &Path) -> Result<(), SettingsError> {
    check_folder(dir).map_err(|e| SettingsError::Invalid(e.to_string()))
}
