//! The signed list of the video tools (ADR-0011, section 2; owner's request,
//! 2026-10-31: yt-dlp updated without a whole release of Zajil).
//!
//! `tools.json` names, for each tool, what `tools/*/pins*.txt` name in a
//! build: its version, and each system's release file with its SHA-256. It
//! comes with a minisign signature by one of the project's two update keys
//! (the same as Zajil's own update, `apps/desktop/src-tauri/update-keys`),
//! and a sequence number. Zajil takes a list only when its signature is
//! good and its number is higher than the one built into it and than the
//! list it kept (never back to an older tool). The list never names a
//! place to download from: each tool's releases are fixed in the code
//! (`tools::from_pins`), so the list can only choose among their files,
//! and each file's SHA-256 is checked as before, before every run.
//!
//! A kept list is in Zajil's data folder (`tools-list.json` and its
//! `tools-list.json.minisig`), written safely, and checked again at every
//! read: changed on the disk, it is ignored, and the tools built in are
//! used.

use std::collections::BTreeMap;
use std::io::Read;
use std::path::Path;

use base64::Engine as _;
use serde::Deserialize;

use crate::tools::{PinsFile, Tool, parse_pins};

/// The list kept in the data folder, and its signature.
pub const LIST_FILE: &str = "tools-list.json";
pub const SIGNATURE_FILE: &str = "tools-list.json.minisig";
/// The most read of either.
const MAX_FILE: u64 = 64 * 1024;

/// The sequence number of the tools built into this Zajil
/// (`tools/sequence.txt`; a test reads it).
pub fn built_in_sequence() -> u64 {
    static SEQUENCE: std::sync::OnceLock<u64> = std::sync::OnceLock::new();
    *SEQUENCE.get_or_init(|| {
        include_str!("../../../tools/sequence.txt")
            .lines()
            .map(str::trim)
            .find(|l| !l.is_empty() && !l.starts_with('#'))
            .and_then(|l| l.parse().ok())
            .expect("tools/sequence.txt holds a number")
    })
}

/// The project's update keys, as Tauri's signer writes them (base64 of
/// minisign's public key file): the signing key, then the offline
/// emergency one. An empty one is none.
const KEYS: [&str; 2] = [
    include_str!("../../../apps/desktop/src-tauri/update-keys/signing.pub"),
    include_str!("../../../apps/desktop/src-tauri/update-keys/emergency.pub"),
];

/// A checked list: its number, when it was made, and each tool's pins.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ToolsList {
    pub sequence: u64,
    pub issued_at: String,
    pins: BTreeMap<&'static str, PinsFile>,
}

/// The names of the tools in the list, as in `tools/`.
const NAMES: [(&str, Tool); 4] = [
    ("yt-dlp", Tool::YtDlp),
    ("quickjs", Tool::QuickJs),
    ("ffmpeg", Tool::Ffmpeg),
    ("ffprobe", Tool::Ffprobe),
];

impl ToolsList {
    /// The pins of `tool` in this list.
    pub fn pins(&self, tool: Tool) -> &PinsFile {
        let name = NAMES
            .iter()
            .find(|(_, t)| *t == tool)
            .expect("every tool")
            .0;
        &self.pins[name]
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Raw {
    format: String,
    sequence: u64,
    issued_at: String,
    tools: BTreeMap<String, String>,
}

/// The list's own format.
const FORMAT: &str = "zajil-tools-1";

/// Reads a list strictly: its format, a number, a time, and the pins of
/// every tool and no other, each as `tools/*/pins*.txt` are read; ffmpeg
/// and ffprobe of one version (they share a folder).
pub fn parse(json: &[u8]) -> Result<ToolsList, String> {
    let raw: Raw = serde_json::from_slice(json).map_err(|e| format!("not a tools list: {e}"))?;
    if raw.format != FORMAT {
        return Err(format!("not a {FORMAT} list"));
    }
    if raw.issued_at.len() > 40 || raw.issued_at.chars().any(char::is_control) {
        return Err("an odd issued_at".into());
    }
    if raw.tools.len() != NAMES.len() {
        return Err("not every tool, or another".into());
    }
    let mut pins = BTreeMap::new();
    for (name, _) in NAMES {
        let text = raw.tools.get(name).ok_or_else(|| format!("no {name}"))?;
        pins.insert(name, parse_pins(text).map_err(|e| format!("{name}: {e}"))?);
    }
    if pins["ffmpeg"].version != pins["ffprobe"].version {
        return Err("ffmpeg and ffprobe of two versions".into());
    }
    Ok(ToolsList {
        sequence: raw.sequence,
        issued_at: raw.issued_at,
        pins,
    })
}

/// Whether `signature` (minisign's text) is of `json` by one of `keys`.
fn signed_by_any(json: &[u8], signature: &str, keys: &[&str]) -> bool {
    let Ok(signature) = minisign_verify::Signature::decode(signature) else {
        return false;
    };
    keys.iter().any(|key| {
        let key = key.trim();
        if key.is_empty() {
            return false;
        }
        let text = base64::engine::general_purpose::STANDARD
            .decode(key)
            .ok()
            .and_then(|b| String::from_utf8(b).ok());
        text.and_then(|t| minisign_verify::PublicKey::decode(&t).ok())
            .is_some_and(|key| key.verify(json, &signature, false).is_ok())
    })
}

/// `json`, signed by `signature` with one of the project's keys, read.
pub fn checked(json: &[u8], signature: &str) -> Result<ToolsList, String> {
    checked_with(json, signature, &KEYS)
}

fn checked_with(json: &[u8], signature: &str, keys: &[&str]) -> Result<ToolsList, String> {
    if !signed_by_any(json, signature, keys) {
        return Err("the tools list is not signed by a key of Zajil's".into());
    }
    parse(json)
}

/// A small file of the data folder, never through a link.
fn read_small(path: &Path) -> std::io::Result<Vec<u8>> {
    let file = zajil_platform::fs::open_regular_nofollow(path, false)?;
    let mut bytes = Vec::new();
    file.take(MAX_FILE).read_to_end(&mut bytes)?;
    Ok(bytes)
}

/// The list kept in `data_dir`, if it is there, still signed, and newer
/// than the tools built in. Blocking.
pub fn kept(data_dir: &Path) -> Option<ToolsList> {
    kept_with(data_dir, &KEYS, built_in_sequence())
}

fn kept_with(data_dir: &Path, keys: &[&str], built_in: u64) -> Option<ToolsList> {
    let json = read_small(&data_dir.join(LIST_FILE)).ok()?;
    let signature = read_small(&data_dir.join(SIGNATURE_FILE)).ok()?;
    let signature = String::from_utf8(signature).ok()?;
    match checked_with(&json, &signature, keys) {
        Ok(list) if list.sequence > built_in => Some(list),
        Ok(_) => None,
        Err(e) => {
            eprintln!("zajil: the kept tools list is not used: {e}");
            None
        }
    }
}

/// Why a list was not kept.
#[derive(Debug, PartialEq, Eq)]
pub enum Refused {
    /// Its signature or its contents.
    Bad(String),
    /// Its number is not higher than this Zajil's or the kept list's.
    NotNewer { theirs: u64, ours: u64 },
}

impl std::fmt::Display for Refused {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Refused::Bad(why) => f.write_str(why),
            Refused::NotNewer { theirs, ours } => {
                write!(f, "the tools list {theirs} is not newer than {ours}")
            }
        }
    }
}

/// Keeps `json` and its `signature` in `data_dir` when they are a signed
/// list newer than the tools this Zajil has (built in or kept). Returns the
/// list. Blocking.
pub fn keep(data_dir: &Path, json: &[u8], signature: &str) -> Result<ToolsList, Refused> {
    keep_with(data_dir, json, signature, &KEYS, built_in_sequence())
}

fn keep_with(
    data_dir: &Path,
    json: &[u8],
    signature: &str,
    keys: &[&str],
    built_in: u64,
) -> Result<ToolsList, Refused> {
    let list = checked_with(json, signature, keys).map_err(Refused::Bad)?;
    let ours = kept_with(data_dir, keys, built_in).map_or(built_in, |k| k.sequence);
    if list.sequence <= ours {
        return Err(Refused::NotNewer {
            theirs: list.sequence,
            ours,
        });
    }
    // The signature last: until it is there, the old one does not match
    // the new list, which is then ignored, never half taken.
    crate::state::write_safely(&data_dir.join(LIST_FILE), json)
        .and_then(|()| {
            crate::state::write_safely(&data_dir.join(SIGNATURE_FILE), signature.as_bytes())
        })
        .map_err(|e| Refused::Bad(format!("could not keep the tools list: {e}")))?;
    Ok(list)
}

/// Where the newest list is published: a release of its own, apart from
/// Zajil's (`.github/workflows/tools-list.yml`), so yt-dlp can be updated
/// without a release of Zajil.
pub const LIST_URL: &str =
    "https://github.com/motalim-ai/zajil-download-manager/releases/download/tools-list/tools.json";
pub const SIGNATURE_URL: &str = "https://github.com/motalim-ai/zajil-download-manager/releases/download/tools-list/tools.json.minisig";

/// Asks for the newest list and keeps it when it is signed and newer:
/// `Some` with it when kept, `None` when it is not newer. The tools then
/// update as an earlier version does (`Toolbox::has_an_earlier_one`: the
/// user agreed to them once).
pub async fn fetch_and_keep(
    engine: &crate::engine::Engine,
    data_dir: &Path,
    stop: &crate::engine::Stop,
) -> Result<Option<ToolsList>, String> {
    let limit = MAX_FILE as usize;
    let json = engine
        .fetch_text(LIST_URL, limit, stop)
        .await
        .map_err(|e| format!("the tools list: {e}"))?;
    let signature = engine
        .fetch_text(SIGNATURE_URL, limit, stop)
        .await
        .map_err(|e| format!("the tools list's signature: {e}"))?;
    let dir = data_dir.to_owned();
    match tokio::task::spawn_blocking(move || keep(&dir, json.as_bytes(), &signature))
        .await
        .map_err(|e| e.to_string())?
    {
        Ok(list) => Ok(Some(list)),
        Err(Refused::NotNewer { .. }) => Ok(None),
        Err(Refused::Bad(why)) => Err(why),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // Two throwaway test keys (`tauri signer generate`, their private
    // halves not kept); tests/fixtures/tools-list is signed with the first.
    const KEY_A: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IEM1MEJBMjUyOTMyODZCNEYKUldSUGF5aVRVcUlMeGIzWHdzZ3czU3FwWWphaXdKOFV1Yjd5cXFKSzZvdnBCVHF0WTRQUkhsU2sK";
    const KEY_B: &str = "dW50cnVzdGVkIGNvbW1lbnQ6IG1pbmlzaWduIHB1YmxpYyBrZXk6IDVGQjMzMzNGMTUzRThBQzcKUldUSGlqNFZQek96WHl5dWJtYitJWmwzZlZ1VHhqYm5qZDVCN1d2MEQ0U2QyUEtyRHlxUjVFOXAK";

    fn pins(version: &str) -> String {
        let sha = "a".repeat(64);
        format!("version {version}\nlinux-x86_64 tool {sha}\n")
    }

    fn list(sequence: u64, yt_dlp: &str) -> String {
        serde_json::json!({
            "format": FORMAT,
            "sequence": sequence,
            "issued_at": "2026-10-31T12:00:00Z",
            "tools": {
                "yt-dlp": pins(yt_dlp),
                "quickjs": pins("v0.17.0"),
                "ffmpeg": pins("8.1.3-1"),
                "ffprobe": pins("8.1.3-1"),
            },
        })
        .to_string()
    }

    /// The built-in number is a number, and the list's form is strict.
    #[test]
    fn a_list_is_read_strictly() {
        assert!(built_in_sequence() >= 1);
        let ok = parse(list(2, "2026.11.01").as_bytes()).unwrap();
        assert_eq!(ok.sequence, 2);
        assert_eq!(ok.pins(Tool::YtDlp).version, "2026.11.01");
        let mut bad: serde_json::Value = serde_json::from_str(&list(2, "x")).unwrap();
        bad["tools"]["ffprobe"] = pins("9").into();
        assert!(
            parse(bad.to_string().as_bytes()).is_err(),
            "two ffmpeg versions"
        );
        let mut extra: serde_json::Value = serde_json::from_str(&list(2, "x")).unwrap();
        extra["tools"]["curl"] = pins("1").into();
        assert!(parse(extra.to_string().as_bytes()).is_err());
        let mut url: serde_json::Value = serde_json::from_str(&list(2, "x")).unwrap();
        url["url"] = "https://evil.example/".into();
        assert!(parse(url.to_string().as_bytes()).is_err(), "no other field");
        let wrong = list(2, "../up");
        assert!(
            parse(wrong.as_bytes()).is_err(),
            "a version is a plain name"
        );
    }

    /// The list the workflow writes (`.github/workflows/tools-list.yml`:
    /// each pins file as it is) is one Zajil reads, and names this build's
    /// tools.
    #[test]
    fn this_build_s_pins_make_a_list() {
        let json = serde_json::json!({
            "format": FORMAT,
            "sequence": built_in_sequence(),
            "issued_at": "2026-10-31T12:00:00Z",
            "tools": {
                "yt-dlp": include_str!("../../../tools/yt-dlp/pins.txt"),
                "quickjs": include_str!("../../../tools/quickjs/pins.txt"),
                "ffmpeg": include_str!("../../../tools/ffmpeg/pins-ffmpeg.txt"),
                "ffprobe": include_str!("../../../tools/ffmpeg/pins-ffprobe.txt"),
            },
        });
        let list = parse(json.to_string().as_bytes()).unwrap();
        assert_eq!(
            list.pins(Tool::YtDlp).version,
            crate::tools::yt_dlp_version()
        );
    }

    /// Only a list signed by a key of ours is taken, and only a newer one
    /// is kept; a kept list changed on the disk is not used.
    #[test]
    fn a_list_is_kept_only_signed_and_newer() {
        // A fixed list signed with key A (tests/fixtures/tools-list).
        let dir = tempfile::tempdir().unwrap();
        let fixture =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/tools-list");
        let json = std::fs::read(fixture.join("tools.json")).unwrap();
        let signature = std::fs::read_to_string(fixture.join("tools.json.minisig")).unwrap();
        // Not by our keys: refused.
        assert!(matches!(
            keep_with(dir.path(), &json, &signature, &[KEY_B], 1),
            Err(Refused::Bad(_))
        ));
        // Not newer than the built-in tools.
        assert_eq!(
            keep_with(dir.path(), &json, &signature, &[KEY_A], 5),
            Err(Refused::NotNewer { theirs: 2, ours: 5 })
        );
        // Newer: kept, and read back.
        let kept = keep_with(dir.path(), &json, &signature, &[KEY_B, KEY_A], 1).unwrap();
        assert_eq!(kept.sequence, 2);
        assert_eq!(kept_with(dir.path(), &[KEY_A], 1), Some(kept));
        // The same again: not newer than the kept one.
        assert!(matches!(
            keep_with(dir.path(), &json, &signature, &[KEY_A], 1),
            Err(Refused::NotNewer { theirs: 2, ours: 2 })
        ));
        // A Zajil that has these tools built in does not use it.
        assert_eq!(kept_with(dir.path(), &[KEY_A], 2), None);
        // Changed on the disk: ignored.
        let mut changed = json.clone();
        let at = changed.iter().position(|&b| b == b'2').unwrap();
        changed[at] = b'9';
        std::fs::write(dir.path().join(LIST_FILE), &changed).unwrap();
        assert_eq!(kept_with(dir.path(), &[KEY_A], 1), None);
    }
}
