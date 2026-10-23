//! The per-download state file (ADR-0004).
//!
//! Each download has two files next to each other in the download folder:
//! `<name>.zajil-part` (the data) and `<name>.zajil-state` (this JSON file).
//! The format is specified in `docs/STATE-FILE.md`.
//!
//! Safe-write rules implemented here:
//! - The file is replaced in one step: write a temporary file, sync it,
//!   rename it over the old one, then sync the folder.
//! - A `checksum` field detects corruption. A corrupt file is rejected; we
//!   never guess.
//! - Only the user who downloads may read the file (mode `0600`, or an
//!   access list for that user alone on Windows): `url` and `final_url` may
//!   hold a token that resuming needs, such as `?sig=…` (owner's decision on
//!   #28). It is never printed.
//!
//! The caller must sync the `.zajil-part` data *before* saving a state that
//! counts it as done.

use std::fs;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};

/// Value of the `format` field.
pub const FORMAT: &str = "zajil-state";
/// The schema this version of Zajil reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// Suffix of the state file.
pub const STATE_SUFFIX: &str = ".zajil-state";
/// Suffix of the partial data file.
pub const PART_SUFFIX: &str = ".zajil-part";

const CHECKSUM_PREFIX: &str = "sha256:";

/// A real state file is a few kilobytes; anything above this is refused.
const MAX_STATE_BYTES: u64 = 1024 * 1024;

/// What we know about one download, enough to resume it later.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct DownloadState {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`SCHEMA_VERSION`] when written by this version.
    pub schema_version: u32,
    /// The URL the user asked for.
    pub url: String,
    /// The URL after redirects. Resuming requests this one first.
    pub final_url: String,
    /// Identifies the version of the file on the server (RFC 9110 §8.8).
    pub validator: Validator,
    /// Size of the whole file, if the server told us.
    pub content_length: Option<u64>,
    /// Whether the server announced `Accept-Ranges: bytes`.
    pub accept_ranges: bool,
    /// Whether the URL the user gave had a user name or password. They are
    /// never saved (ADR-0004, rule 7); this only says that a `401` while
    /// resuming means they are needed again. Absent in files written before
    /// it existed, which counts as `false`.
    #[serde(default)]
    pub had_credentials: bool,
    /// Byte ranges of the file, in order and without gaps, and how much of
    /// each is on disk. One segment for a single connection; one per
    /// connection, then more as segments are split, for a parallel download.
    pub segments: Vec<Segment>,
    /// RFC 3339 time in UTC.
    pub created_at: String,
    /// RFC 3339 time in UTC.
    pub updated_at: String,
    /// `sha256:` and the hex digest of the file with this field empty.
    pub checksum: String,
}

/// Server validators for `If-Range`.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Validator {
    pub etag: Option<String>,
    pub last_modified: Option<String>,
    /// The `Date` header of the same response. `last_modified` is a strong
    /// validator only if it is at least one second before this date
    /// (RFC 9110 §8.8.2.2).
    pub date: Option<String>,
}

/// A byte range of the file. `end` is inclusive and `None` when the size is
/// unknown. Bytes `start .. start + done` are on disk and synced.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Segment {
    pub start: u64,
    pub end: Option<u64>,
    pub done: u64,
}

/// Why a state file could not be used.
#[derive(Debug, thiserror::Error)]
pub enum StateError {
    #[error("could not read or write the state file: {0}")]
    Io(#[from] io::Error),
    #[error("this is not a Zajil state file")]
    NotAStateFile,
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
    #[error("the state file uses schema version {0}, which this version of Zajil cannot read")]
    UnsupportedVersion(u64),
    #[error("the state file is damaged and cannot be used: {0}")]
    Corrupt(String),
    #[error("the state file is damaged and cannot be used: its checksum does not match")]
    ChecksumMismatch,
}

impl DownloadState {
    /// A fresh state for a download that has not written any data yet.
    pub fn new(
        url: String,
        final_url: String,
        validator: Validator,
        content_length: Option<u64>,
        accept_ranges: bool,
    ) -> Self {
        let now = now_rfc3339();
        DownloadState {
            format: FORMAT.to_owned(),
            schema_version: SCHEMA_VERSION,
            url,
            final_url,
            validator,
            content_length,
            accept_ranges,
            had_credentials: false,
            segments: vec![Segment {
                start: 0,
                end: content_length.and_then(|len| len.checked_sub(1)),
                done: 0,
            }],
            created_at: now.clone(),
            updated_at: now,
            checksum: String::new(),
        }
    }

    /// Bytes on disk, summed over all segments. Saturates instead of
    /// overflowing on a state that was never checked.
    pub fn bytes_done(&self) -> u64 {
        self.segments
            .iter()
            .fold(0u64, |sum, s| sum.saturating_add(s.done))
    }

    /// Reads and fully checks a state file. The file itself is untrusted
    /// input: it may come from another machine (ADR-0004), so a link, an
    /// oversized file or any inconsistency is refused.
    pub fn load(path: &Path) -> Result<Self, StateError> {
        let file = zajil_platform::fs::open_regular_nofollow(path, false).map_err(|err| {
            if zajil_platform::fs::is_not_regular_file(&err) {
                StateError::NotRegularFile(path.to_path_buf())
            } else {
                StateError::Io(err)
            }
        })?;
        let mut bytes = Vec::new();
        file.take(MAX_STATE_BYTES + 1).read_to_end(&mut bytes)?;
        if bytes.len() as u64 > MAX_STATE_BYTES {
            return Err(StateError::Corrupt("the file is too large".into()));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| StateError::Corrupt(e.to_string()))?;

        if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
            return Err(StateError::NotAStateFile);
        }
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(SCHEMA_VERSION) => {}
            Some(v) => return Err(StateError::UnsupportedVersion(v)),
            None => return Err(StateError::Corrupt("missing schema_version".into())),
        }

        // The checksum covers the file as written, in canonical form, so it
        // does not depend on how this version of the code orders fields.
        let Some(stored) = value.get("checksum").and_then(|v| v.as_str()) else {
            return Err(StateError::Corrupt("missing checksum".into()));
        };
        if stored != checksum_of(&value) {
            return Err(StateError::ChecksumMismatch);
        }
        let state: DownloadState =
            serde_json::from_value(value).map_err(|e| StateError::Corrupt(e.to_string()))?;
        state.check_consistency()?;
        Ok(state)
    }

    /// Writes the state to `path` in one step, replacing any previous file.
    /// Updates `updated_at` and `checksum`.
    pub fn save(&mut self, path: &Path) -> Result<(), StateError> {
        self.updated_at = now_rfc3339();
        self.checksum = self.compute_checksum();
        let mut json = serde_json::to_vec_pretty(self).expect("state serializes");
        json.push(b'\n');

        let tmp = temporary_path(path);
        // A leftover temporary file, or a link planted in its place, is
        // removed first. Removing a link never touches what it points to,
        // and `create_new` then refuses anything that reappears.
        match fs::remove_file(&tmp) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err.into()),
        }
        let written = (|| {
            // Only the user may read it: the URL may hold a token that
            // resuming needs (`?sig=…`, owner's decision on #28).
            let mut file = zajil_platform::fs::create_private(&tmp)?;
            file.write_all(&json)?;
            file.sync_all()?;
            drop(file);
            #[cfg(feature = "test-support")]
            test_hooks::after_temporary_written(&tmp);
            zajil_platform::fs::replace_file(&tmp, path)
        })();
        if let Err(err) = written {
            let _ = fs::remove_file(&tmp);
            return Err(err.into());
        }
        let folder = match path.parent() {
            Some(p) if !p.as_os_str().is_empty() => p,
            _ => Path::new("."),
        };
        zajil_platform::fs::sync_dir(folder)?;
        Ok(())
    }

    fn compute_checksum(&self) -> String {
        checksum_of(&serde_json::to_value(self).expect("state serializes"))
    }

    /// Rejects values that cannot describe a real download, even when the
    /// checksum matches (for example a file written by a buggy tool). The
    /// segments must cover the file from byte 0 without gaps or overlaps, in
    /// order; with an unknown size there is exactly one open-ended segment.
    /// The numbers are untrusted, so no arithmetic here can overflow.
    fn check_consistency(&self) -> Result<(), StateError> {
        let bad = |why: &str| Err(StateError::Corrupt(why.to_owned()));
        let Some(first) = self.segments.first() else {
            return bad("no segments");
        };
        if first.start != 0 {
            return bad("the first segment does not start at byte 0");
        }
        let Some(len) = self.content_length else {
            return match self.segments.as_slice() {
                [Segment { end: None, .. }] => Ok(()),
                _ => bad("a file of unknown size must have one open-ended segment"),
            };
        };
        if len == 0 {
            // An empty file has no byte to describe (`DownloadState::new`).
            return match self.segments.as_slice() {
                [
                    Segment {
                        end: None, done: 0, ..
                    },
                ] => Ok(()),
                _ => bad("an empty file must have one empty segment"),
            };
        }
        let mut next = 0u64;
        for segment in &self.segments {
            let Some(end) = segment.end else {
                return bad("a segment of a file of known size has no end");
            };
            if segment.start != next {
                return bad("the segments leave a gap or overlap");
            }
            if end < segment.start {
                return bad("a segment ends before it starts");
            }
            if end >= len {
                return bad("a segment goes past the end of the file");
            }
            // `end < len <= u64::MAX`, so `end - start + 1` cannot overflow.
            if segment.done > end - segment.start + 1 {
                return bad("a segment has more bytes done than its size");
            }
            next = end + 1;
        }
        if next != len {
            return bad("the segments do not reach the end of the file");
        }
        Ok(())
    }
}

/// `sha256:` and the hex SHA-256 of the canonical form of `value` with its
/// `checksum` member set to the empty string.
fn checksum_of(value: &serde_json::Value) -> String {
    let mut unsigned = value.clone();
    if let Some(object) = unsigned.as_object_mut() {
        object.insert("checksum".into(), serde_json::Value::String(String::new()));
    }
    let mut canonical = String::new();
    write_canonical(&unsigned, &mut canonical);
    let digest = Sha256::digest(canonical.as_bytes());
    let mut out = String::with_capacity(CHECKSUM_PREFIX.len() + 64);
    out.push_str(CHECKSUM_PREFIX);
    for byte in digest.iter() {
        out.push_str(&format!("{byte:02x}"));
    }
    out
}

/// The canonical form (docs/STATE-FILE.md): compact JSON, no spaces, the
/// members of every object sorted by key in code-point order, strings and
/// numbers as `serde_json` writes them. The sort is done here explicitly, so
/// it holds whatever map type `serde_json` is built with.
fn write_canonical(value: &serde_json::Value, out: &mut String) {
    use serde_json::Value;
    match value {
        Value::Array(items) => {
            out.push('[');
            for (i, item) in items.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                write_canonical(item, out);
            }
            out.push(']');
        }
        Value::Object(members) => {
            let mut keys: Vec<&String> = members.keys().collect();
            keys.sort();
            out.push('{');
            for (i, key) in keys.into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push_str(&serde_json::to_string(key).expect("string serializes"));
                out.push(':');
                write_canonical(&members[key], out);
            }
            out.push('}');
        }
        scalar => out.push_str(&serde_json::to_string(scalar).expect("scalar serializes")),
    }
}

/// **Test builds only** (`test-support` feature): a hook called right after
/// the temporary state file is written and closed, before it replaces the
/// state file. That is the moment an antivirus scans a new file, so tests
/// use it to hold the file the way a scanner does.
#[cfg(feature = "test-support")]
pub mod test_hooks {
    use std::path::Path;
    use std::sync::Mutex;

    type Hook = Box<dyn Fn(&Path) + Send + Sync>;

    static AFTER_TEMPORARY_WRITTEN: Mutex<Option<Hook>> = Mutex::new(None);

    /// Sets (or with `None`, clears) the hook. It is global: tests that use
    /// it must run in their own test binary.
    pub fn set_after_temporary_written(hook: Option<Hook>) {
        *AFTER_TEMPORARY_WRITTEN
            .lock()
            .unwrap_or_else(|e| e.into_inner()) = hook;
    }

    pub(crate) fn after_temporary_written(tmp: &Path) {
        if let Some(hook) = &*AFTER_TEMPORARY_WRITTEN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
        {
            hook(tmp);
        }
    }
}

/// `<folder>/<name>.zajil-state` for the final file `<folder>/<name>`.
pub fn state_path(final_path: &Path) -> PathBuf {
    with_suffix(final_path, STATE_SUFFIX)
}

/// `<folder>/<name>.zajil-part` for the final file `<folder>/<name>`.
pub fn part_path(final_path: &Path) -> PathBuf {
    with_suffix(final_path, PART_SUFFIX)
}

/// The final file for a state file, or `None` if `state` does not end with
/// [`STATE_SUFFIX`].
pub fn final_path_for_state(state: &Path) -> Option<PathBuf> {
    let name = state.file_name()?.to_str()?;
    let stem = name.strip_suffix(STATE_SUFFIX)?;
    if stem.is_empty() {
        return None;
    }
    Some(state.with_file_name(stem))
}

fn with_suffix(path: &Path, suffix: &str) -> PathBuf {
    let mut name = path.file_name().unwrap_or_default().to_os_string();
    name.push(suffix);
    path.with_file_name(name)
}

fn temporary_path(path: &Path) -> PathBuf {
    with_suffix(path, ".tmp")
}

fn now_rfc3339() -> String {
    rfc3339_utc(SystemTime::now())
}

/// Formats a time as `YYYY-MM-DDTHH:MM:SSZ`.
fn rfc3339_utc(time: SystemTime) -> String {
    let secs = time
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);
    let (days, rem) = (secs / 86_400, secs % 86_400);
    let (year, month, day) = civil_from_days(days as i64);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rem / 3600,
        rem % 3600 / 60,
        rem % 60
    )
}

/// Days since 1970-01-01 to a proleptic Gregorian date (Howard Hinnant's
/// public-domain `civil_from_days` algorithm).
fn civil_from_days(days: i64) -> (i64, u32, u32) {
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z.rem_euclid(146_097);
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let month = if mp < 10 { mp + 3 } else { mp - 9 } as u32;
    let year = yoe + era * 400 + i64::from(month <= 2);
    (year, month, day)
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::*;

    #[test]
    fn the_canonical_form_sorts_keys_at_every_level() {
        let value = serde_json::json!({"b": 1, "a": {"z": [ {"y": 1, "x": "\"q\""} ], "c": null}});
        let mut out = String::new();
        write_canonical(&value, &mut out);
        assert_eq!(out, r#"{"a":{"c":null,"z":[{"x":"\"q\"","y":1}]},"b":1}"#);
    }

    #[test]
    fn formats_utc_times() {
        assert_eq!(rfc3339_utc(UNIX_EPOCH), "1970-01-01T00:00:00Z");
        let t = UNIX_EPOCH + Duration::from_secs(1_700_000_000);
        assert_eq!(rfc3339_utc(t), "2023-11-14T22:13:20Z");
        let leap = UNIX_EPOCH + Duration::from_secs(951_782_400);
        assert_eq!(rfc3339_utc(leap), "2000-02-29T00:00:00Z");
    }

    #[test]
    fn new_state_has_one_segment_covering_the_file() {
        let state =
            DownloadState::new("u".into(), "u".into(), Validator::default(), Some(10), true);
        assert_eq!(
            state.segments,
            vec![Segment {
                start: 0,
                end: Some(9),
                done: 0
            }]
        );
        let unknown = DownloadState::new("u".into(), "u".into(), Validator::default(), None, false);
        assert_eq!(unknown.segments[0].end, None);
    }

    #[test]
    fn derives_companion_paths() {
        let final_path = Path::new("dl").join("a.zip");
        assert_eq!(
            state_path(&final_path),
            Path::new("dl").join("a.zip.zajil-state")
        );
        assert_eq!(
            part_path(&final_path),
            Path::new("dl").join("a.zip.zajil-part")
        );
        assert_eq!(
            final_path_for_state(&state_path(&final_path)),
            Some(final_path)
        );
        assert_eq!(final_path_for_state(Path::new("a.zip")), None);
        assert_eq!(final_path_for_state(Path::new(".zajil-state")), None);
    }
}
