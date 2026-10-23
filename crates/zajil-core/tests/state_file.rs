//! The state file survives a round trip and refuses damaged input.

use std::fs;
use std::path::{Path, PathBuf};

use sha2::Digest;
use zajil_core::state::{DownloadState, StateError, Validator, state_path};

fn sample() -> DownloadState {
    let mut state = DownloadState::new(
        "http://127.0.0.1/file.bin".into(),
        "http://127.0.0.1/cdn/file.bin".into(),
        Validator {
            etag: Some("\"v1\"".into()),
            last_modified: Some("Wed, 21 Oct 2026 07:28:00 GMT".into()),
            date: Some("Wed, 21 Oct 2026 08:00:00 GMT".into()),
        },
        Some(1000),
        true,
    );
    state.segments[0].done = 300;
    state
}

fn saved_sample(dir: &Path) -> (PathBuf, DownloadState) {
    let path = state_path(&dir.join("file.bin"));
    let mut state = sample();
    state.save(&path).unwrap();
    (path, state)
}

/// Rewrites the file through a JSON value, without fixing the checksum.
fn edit(path: &Path, change: impl FnOnce(&mut serde_json::Value)) {
    let mut value: serde_json::Value = serde_json::from_slice(&fs::read(path).unwrap()).unwrap();
    change(&mut value);
    fs::write(path, serde_json::to_vec_pretty(&value).unwrap()).unwrap();
}

#[test]
fn round_trips() {
    let dir = tempfile::tempdir().unwrap();
    let (path, state) = saved_sample(dir.path());
    let loaded = DownloadState::load(&path).unwrap();
    assert_eq!(loaded, state);
    assert_eq!(loaded.bytes_done(), 300);
    assert!(loaded.checksum.starts_with("sha256:"));
}

#[test]
fn saving_again_replaces_the_file_and_leaves_no_temporary_file() {
    let dir = tempfile::tempdir().unwrap();
    let (path, mut state) = saved_sample(dir.path());
    state.segments[0].done = 900;
    state.save(&path).unwrap();
    assert_eq!(DownloadState::load(&path).unwrap().bytes_done(), 900);

    let names: Vec<_> = fs::read_dir(dir.path())
        .unwrap()
        .map(|e| e.unwrap().file_name().into_string().unwrap())
        .collect();
    assert_eq!(names, vec!["file.bin.zajil-state"]);
}

#[test]
fn rejects_a_changed_value_with_a_stale_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| v["segments"][0]["done"] = 999.into());
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::ChecksumMismatch)
    ));
}

#[test]
fn rejects_a_wrong_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| {
        v["checksum"] =
            "sha256:0000000000000000000000000000000000000000000000000000000000000000".into()
    });
    let err = DownloadState::load(&path).unwrap_err();
    assert!(matches!(err, StateError::ChecksumMismatch));
    assert!(err.to_string().contains("damaged"));
}

#[test]
fn rejects_a_missing_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| {
        v.as_object_mut().unwrap().remove("checksum");
    });
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::Corrupt(_))
    ));
}

#[test]
fn rejects_a_truncated_file() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    let bytes = fs::read(&path).unwrap();
    fs::write(&path, &bytes[..bytes.len() / 2]).unwrap();
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::Corrupt(_))
    ));
}

#[test]
fn rejects_garbage_and_empty_files() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.zajil-state");
    for content in [&b""[..], b"\0\0\0\0", b"not json", b"[]"] {
        fs::write(&path, content).unwrap();
        assert!(
            matches!(
                DownloadState::load(&path),
                Err(StateError::Corrupt(_) | StateError::NotAStateFile)
            ),
            "{content:?} was accepted"
        );
    }
}

#[test]
fn rejects_other_json_files() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| v["format"] = "something-else".into());
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::NotAStateFile)
    ));
}

#[test]
fn rejects_an_unknown_schema_version() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| v["schema_version"] = 2.into());
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::UnsupportedVersion(2))
    ));
}

#[test]
fn rejects_unknown_fields_even_with_a_valid_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    edit(&path, |v| {
        v["cookies"] = "secret".into();
        // Re-sign it as docs/STATE-FILE.md says: compact JSON with sorted
        // keys (serde_json's default map is sorted) and an empty checksum.
        v["checksum"] = "".into();
        let canonical = serde_json::to_string(v).unwrap();
        let digest = sha2::Sha256::digest(canonical.as_bytes());
        let hex: String = digest.iter().map(|b| format!("{b:02x}")).collect();
        v["checksum"] = format!("sha256:{hex}").into();
    });
    let err = DownloadState::load(&path).unwrap_err();
    assert!(matches!(err, StateError::Corrupt(_)), "{err}");
    assert!(err.to_string().contains("cookies"), "{err}");
}

#[test]
fn rejects_impossible_segments_even_with_a_valid_checksum() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.zajil-state");
    let mut state = sample();
    state.segments[0].done = 5000; // more than the 1000-byte file
    state.save(&path).unwrap();
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::Corrupt(_))
    ));
}

#[test]
fn a_missing_file_is_an_io_error() {
    let dir = tempfile::tempdir().unwrap();
    assert!(matches!(
        DownloadState::load(&dir.path().join("none.zajil-state")),
        Err(StateError::Io(_))
    ));
}

#[test]
fn the_file_is_readable_json() {
    let dir = tempfile::tempdir().unwrap();
    let (path, _) = saved_sample(dir.path());
    let text = fs::read_to_string(&path).unwrap();
    assert!(text.contains("\"format\": \"zajil-state\""));
    assert!(text.contains("\"schema_version\": 1"));
}

/// Creates a symbolic link, or reports that this machine cannot. On CI
/// (where `CI` is set) that is a failure, so the test cannot silently stop
/// testing anything.
fn symlinks_available(target: &Path, link: &Path) -> bool {
    match zajil_platform::fs::symlink_file(target, link) {
        Ok(()) => true,
        Err(err) if std::env::var_os("CI").is_none() => {
            eprintln!("skipping: cannot create symbolic links here: {err}");
            false
        }
        Err(err) => panic!("CI must be able to create symbolic links: {err}"),
    }
}

#[test]
fn a_link_planted_at_the_temporary_path_is_not_followed() {
    // Review of #13: `<name>.zajil-state.tmp` as a link to a file outside
    // the download folder must not be truncated or overwritten.
    let root = tempfile::tempdir().unwrap();
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();
    let victim = root.path().join("victim.txt");
    fs::write(&victim, b"precious").unwrap();
    let path = state_path(&dir.join("file.bin"));
    let tmp = dir.join("file.bin.zajil-state.tmp");
    if !symlinks_available(&victim, &tmp) {
        return;
    }

    let mut state = sample();
    state.save(&path).unwrap();

    assert_eq!(fs::read(&victim).unwrap(), b"precious");
    assert_eq!(DownloadState::load(&path).unwrap(), state);
    assert!(fs::symlink_metadata(&tmp).is_err(), "the link is gone");
}

#[test]
fn refuses_a_state_file_that_is_a_symbolic_link() {
    let root = tempfile::tempdir().unwrap();
    let (real, _) = saved_sample(root.path());
    let dir = root.path().join("downloads");
    fs::create_dir(&dir).unwrap();
    let link = state_path(&dir.join("file.bin"));
    if !symlinks_available(&real, &link) {
        return;
    }
    assert!(matches!(
        DownloadState::load(&link),
        Err(StateError::NotRegularFile(_))
    ));
}

#[test]
fn refuses_an_oversized_state_file() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("x.zajil-state");
    fs::write(&path, vec![b' '; 2 * 1024 * 1024]).unwrap();
    let err = DownloadState::load(&path).unwrap_err();
    assert!(err.to_string().contains("too large"), "{err}");
}

/// A v1 state file kept in the repository. It was produced from
/// docs/STATE-FILE.md alone (by a separate script, not by this code), with
/// its members in an order unlike the struct's. Every version of Zajil that
/// reads schema v1 must accept it unchanged.
const GOLDEN_V1: &str = include_str!("fixtures/state-v1.golden.json");

#[test]
fn the_golden_v1_file_is_always_accepted() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.pdf.zajil-state");
    fs::write(&path, GOLDEN_V1).unwrap();

    let state = DownloadState::load(&path).expect("the golden v1 file must load");
    assert_eq!(state.url, "https://example.com/files/report.pdf");
    assert_eq!(state.final_url, "https://cdn.example.com/a/report.pdf");
    assert_eq!(state.validator.etag.as_deref(), Some("\"abc123\""));
    assert_eq!(
        state.validator.date.as_deref(),
        Some("Wed, 21 Oct 2026 08:00:00 GMT")
    );
    assert_eq!(state.content_length, Some(104_857_600));
    assert_eq!(state.bytes_done(), 31_457_280);
    assert_eq!(state.segments[0].end, Some(104_857_599));
}

#[test]
fn the_golden_file_still_detects_a_change() {
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("report.pdf.zajil-state");
    fs::write(&path, GOLDEN_V1.replace("31457280", "31457281")).unwrap();
    assert!(matches!(
        DownloadState::load(&path),
        Err(StateError::ChecksumMismatch)
    ));
}

/// Writes JSON with every object's members in reverse order of their keys.
fn reversed_json(value: &serde_json::Value) -> String {
    match value {
        serde_json::Value::Object(map) => {
            let mut keys: Vec<&String> = map.keys().collect();
            keys.sort();
            keys.reverse();
            let members: Vec<String> = keys
                .into_iter()
                .map(|k| {
                    format!(
                        "{}: {}",
                        serde_json::to_string(k).unwrap(),
                        reversed_json(&map[k])
                    )
                })
                .collect();
            format!("{{\n{}\n}}", members.join(",\n"))
        }
        serde_json::Value::Array(items) => {
            let items: Vec<String> = items.iter().map(reversed_json).collect();
            format!("[{}]", items.join(", "))
        }
        scalar => serde_json::to_string(scalar).unwrap(),
    }
}

#[test]
fn the_checksum_does_not_depend_on_member_order_or_spacing() {
    // Review of #13: the checksum used the struct's field order, so
    // reordering fields in the code would have broken every saved file.
    let dir = tempfile::tempdir().unwrap();
    let (path, state) = saved_sample(dir.path());
    let value: serde_json::Value = serde_json::from_slice(&fs::read(&path).unwrap()).unwrap();
    fs::write(&path, reversed_json(&value)).unwrap();
    assert_eq!(DownloadState::load(&path).unwrap(), state);
}
