//! Zajil for Android's own update (ADR-0012, section 8; A4): the model of
//! ADR-0011. `latest-android.json` on the `android-channel` pre-release
//! names the newest version and its APK's SHA-256, and is signed by one of
//! the project's two update keys (minisign). The app takes an update only
//! from a good signature, only to a higher version code (never back), and
//! only the APK file of that version from that version's own release,
//! checked against the SHA-256 here. Android then checks the APK's own
//! signature against the installed app's: a second key, kept apart.

use serde::Deserialize;

use crate::EngineError;

const FORMAT: &str = "zajil-android-update-1";

/// The most an APK may weigh (it is about 25 MB today).
const MAX_APK: u64 = 300 * 1024 * 1024;

/// A checked update.
#[derive(Debug, Clone, PartialEq, Eq, uniffi::Record)]
pub struct AndroidUpdate {
    /// `X.Y.Z`, as the tag `android-vX.Y.Z`.
    pub version: String,
    pub version_code: u64,
    /// The APK's file name in that release: `zajil-android-X.Y.Z-arm64-v8a.apk`.
    pub apk: String,
    pub sha256: String,
    pub size: u64,
    /// What changed, for the user (plain text).
    pub notes: String,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: String,
    version: String,
    version_code: u64,
    apk: String,
    sha256: String,
    size: u64,
    #[serde(default)]
    notes: String,
}

/// `json`, signed by `signature`, read, and newer than `current_version_code`.
///
/// `test_key` is one more key to accept besides the project's: only debug
/// builds pass one (for the emulator test, with a key made for it); a
/// release build's is a compile-time empty constant, so it never gets here.
#[uniffi::export]
pub fn check_android_update(
    json: Vec<u8>,
    signature: String,
    current_version_code: u64,
    test_key: Option<String>,
) -> Result<AndroidUpdate, EngineError> {
    let test_key = test_key.filter(|k| !k.trim().is_empty());
    checked(
        &json,
        &signature,
        current_version_code,
        |json, signature| {
            zajil_core::tools_list::signed_by_project(json, signature)
                || test_key.as_deref().is_some_and(|key| {
                    zajil_core::tools_list::signed_by_any(json, signature, &[key])
                })
        },
    )
}

/// [`check_android_update`] with the signature checked by `signed`.
#[doc(hidden)]
pub fn checked(
    json: &[u8],
    signature: &str,
    current_version_code: u64,
    signed: impl Fn(&[u8], &str) -> bool,
) -> Result<AndroidUpdate, EngineError> {
    let refuse = |why: &str| EngineError::Setup {
        detail: why.to_owned(),
    };
    if json.len() > 64 * 1024 || signature.len() > 4 * 1024 {
        return Err(refuse("the update file is too long"));
    }
    if !signed(json, signature) {
        return Err(refuse("the update is not signed by a key of Zajil's"));
    }
    let raw: Raw =
        serde_json::from_slice(json).map_err(|e| refuse(&format!("not an update file: {e}")))?;
    if raw.format != FORMAT {
        return Err(refuse("not an Android update file"));
    }
    if !is_version(&raw.version) {
        return Err(refuse("the version is not X.Y.Z"));
    }
    if raw.version_code <= current_version_code {
        return Err(refuse("not newer than this Zajil"));
    }
    if raw.apk != format!("zajil-android-{}-arm64-v8a.apk", raw.version) {
        return Err(refuse("not the APK of that version"));
    }
    let hex = raw.sha256.len() == 64
        && raw
            .sha256
            .chars()
            .all(|c| c.is_ascii_digit() || ('a'..='f').contains(&c));
    if !hex {
        return Err(refuse("not a lower-case SHA-256"));
    }
    if raw.size == 0 || raw.size > MAX_APK {
        return Err(refuse("an odd size"));
    }
    if raw.notes.chars().count() > 2000 || raw.notes.chars().any(|c| c.is_control() && c != '\n') {
        return Err(refuse("odd notes"));
    }
    Ok(AndroidUpdate {
        version: raw.version,
        version_code: raw.version_code,
        apk: raw.apk,
        sha256: raw.sha256,
        size: raw.size,
        notes: raw.notes,
    })
}

/// `X.Y.Z`, each a number without a leading zero.
fn is_version(text: &str) -> bool {
    let parts: Vec<&str> = text.split('.').collect();
    parts.len() == 3
        && parts.iter().all(|p| {
            !p.is_empty()
                && p.len() <= 6
                && p.chars().all(|c| c.is_ascii_digit())
                && (p.len() == 1 || !p.starts_with('0'))
        })
}
