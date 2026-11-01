//! The Android app's own signed list of yt-dlp (ADR-0012, section 8;
//! owner's request: yt-dlp ships in the APK, and is updated from a signed
//! list with the project's keys).
//!
//! The desktop's `tools.json` cannot carry it: Zajil 1.2.2 refuses a list
//! with an unknown target or tool. So Android has `tools-android.json`, in
//! the same way: signed by one of the project's two update keys (minisign),
//! with a sequence number that only grows, and never a place to download
//! from: the app takes yt-dlp's pure-Python release file, `yt-dlp`, from
//! yt-dlp's own GitHub release of that version, and checks its SHA-256.

use serde::Deserialize;

use crate::EngineError;

/// The list's own format.
const FORMAT: &str = "zajil-android-tools-1";

/// The release file of yt-dlp that runs in the app's Python.
const YT_DLP_FILE: &str = "yt-dlp";

/// A checked list.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AndroidTools {
    pub sequence: u64,
    pub issued_at: String,
    pub yt_dlp_version: String,
    /// SHA-256 of yt-dlp's release file `yt-dlp`, lower-case hex.
    pub yt_dlp_sha256: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: String,
    sequence: u64,
    issued_at: String,
    yt_dlp: RawYtDlp,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct RawYtDlp {
    version: String,
    file: String,
    sha256: String,
}

/// `json`, signed by `signature` with one of the project's keys, read, and
/// newer than `newer_than` (the list built in or kept): never back to an
/// older yt-dlp.
#[uniffi::export]
pub fn check_android_tools(
    json: Vec<u8>,
    signature: String,
    newer_than: u64,
) -> Result<AndroidTools, EngineError> {
    checked(&json, &signature, newer_than, |json, signature| {
        zajil_core::tools_list::signed_by_project(json, signature)
    })
}

/// [`check_android_tools`] with the signature checked by `signed` (tests
/// use a key of their own).
#[doc(hidden)]
pub fn checked(
    json: &[u8],
    signature: &str,
    newer_than: u64,
    signed: impl Fn(&[u8], &str) -> bool,
) -> Result<AndroidTools, EngineError> {
    let refuse = |why: &str| EngineError::Setup {
        detail: why.to_owned(),
    };
    if json.len() > 64 * 1024 || signature.len() > 4 * 1024 {
        return Err(refuse("the tools list is too long"));
    }
    if !signed(json, signature) {
        return Err(refuse("the tools list is not signed by a key of Zajil's"));
    }
    let raw: Raw =
        serde_json::from_slice(json).map_err(|e| refuse(&format!("not a tools list: {e}")))?;
    if raw.format != FORMAT {
        return Err(refuse("not an Android tools list"));
    }
    if raw.sequence <= newer_than {
        return Err(refuse("not newer than the tools list Zajil has"));
    }
    if raw.issued_at.len() > 40 || raw.issued_at.chars().any(char::is_control) {
        return Err(refuse("an odd issued_at"));
    }
    let y = raw.yt_dlp;
    let plain = !y.version.is_empty()
        && y.version.len() <= 40
        && y.version
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '.' || c == '-');
    if !plain {
        return Err(refuse("the yt-dlp version is not a plain name"));
    }
    if y.file != YT_DLP_FILE {
        return Err(refuse("not yt-dlp's Python release file"));
    }
    let hex = y.sha256.len() == 64
        && y.sha256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !hex {
        return Err(refuse("not a lower-case SHA-256"));
    }
    Ok(AndroidTools {
        sequence: raw.sequence,
        issued_at: raw.issued_at,
        yt_dlp_version: y.version,
        yt_dlp_sha256: y.sha256,
    })
}
