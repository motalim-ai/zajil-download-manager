//! The Android app's signed list of its protection lists (ADR-0012,
//! section 17): ads, trackers, malware and phishing hosts, compiled from
//! open filter lists by the release workflow. The app ships a copy of each,
//! and updates them only from this file: signed by one of the project's
//! two update keys (minisign), with a sequence number that only grows, each
//! list named on the `android-channel` pre-release with its SHA-256 and
//! size. Nothing unsigned is ever fetched.
//!
//! A file of its own, not `tools-android.json`: that list refuses unknown
//! fields, so adding lists to it would stop yt-dlp's updates on 0.2.0.

use serde::Deserialize;

use crate::EngineError;

/// The file's own format.
const FORMAT: &str = "zajil-android-lists-1";

/// The lists the app knows, in its order.
const NAMES: [&str; 4] = ["ads", "trackers", "malware", "phishing"];

/// The largest compressed list the app downloads.
pub const MAX_LIST_SIZE: u64 = 8 * 1024 * 1024;

/// A checked file.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AndroidLists {
    pub sequence: u64,
    pub issued_at: String,
    pub lists: Vec<ListFile>,
}

/// One list: its name, its file on `android-channel`, its SHA-256 and size.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct ListFile {
    pub name: String,
    pub file: String,
    pub sha256: String,
    pub size: u64,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: String,
    sequence: u64,
    issued_at: String,
    lists: Vec<RawList>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawList {
    name: String,
    file: String,
    sha256: String,
    size: u64,
}

/// `json`, signed by `signature` with one of the project's keys, read, and
/// newer than `newer_than` (the lists built in or kept).
#[uniffi::export]
pub fn check_android_lists(
    json: Vec<u8>,
    signature: String,
    newer_than: u64,
) -> Result<AndroidLists, EngineError> {
    checked(&json, &signature, newer_than, |json, signature| {
        zajil_core::tools_list::signed_by_project(json, signature)
    })
}

/// [`check_android_lists`] with the signature checked by `signed` (tests
/// use a key of their own).
#[doc(hidden)]
pub fn checked(
    json: &[u8],
    signature: &str,
    newer_than: u64,
    signed: impl Fn(&[u8], &str) -> bool,
) -> Result<AndroidLists, EngineError> {
    let refuse = |why: &str| EngineError::Setup {
        detail: why.to_owned(),
    };
    if json.len() > 16 * 1024 || signature.len() > 4 * 1024 {
        return Err(refuse("the lists file is too long"));
    }
    if !signed(json, signature) {
        return Err(refuse("the lists file is not signed by a key of Zajil's"));
    }
    let raw: Raw =
        serde_json::from_slice(json).map_err(|e| refuse(&format!("not a lists file: {e}")))?;
    if raw.format != FORMAT {
        return Err(refuse("not an Android lists file"));
    }
    if raw.sequence <= newer_than {
        return Err(refuse("not newer than the lists Zajil has"));
    }
    if raw.issued_at.len() > 40 || raw.issued_at.chars().any(char::is_control) {
        return Err(refuse("an odd issued_at"));
    }
    if raw.lists.is_empty() || raw.lists.len() > NAMES.len() {
        return Err(refuse("not the lists Zajil knows"));
    }
    let mut lists = Vec::with_capacity(raw.lists.len());
    for list in raw.lists {
        if !NAMES.contains(&list.name.as_str())
            || lists.iter().any(|l: &ListFile| l.name == list.name)
        {
            return Err(refuse("a list Zajil does not know, or twice"));
        }
        // «zajil-lists-<name>-<digits>.txt.gz»: a plain file name on the
        // channel, never a path or another place.
        let plain = list
            .file
            .strip_prefix(&format!("zajil-lists-{}-", list.name))
            .and_then(|rest| rest.strip_suffix(".txt.gz"))
            .is_some_and(|n| {
                !n.is_empty() && n.len() <= 20 && n.chars().all(|c| c.is_ascii_digit())
            });
        if !plain {
            return Err(refuse("not a list file's name"));
        }
        let hex = list.sha256.len() == 64
            && list
                .sha256
                .chars()
                .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
        if !hex {
            return Err(refuse("not a lower-case SHA-256"));
        }
        if list.size == 0 || list.size > MAX_LIST_SIZE {
            return Err(refuse("a list of an odd size"));
        }
        lists.push(ListFile {
            name: list.name,
            file: list.file,
            sha256: list.sha256,
            size: list.size,
        });
    }
    Ok(AndroidLists {
        sequence: raw.sequence,
        issued_at: raw.issued_at,
        lists,
    })
}
