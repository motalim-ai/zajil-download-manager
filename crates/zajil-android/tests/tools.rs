//! The Android tools list (ADR-0012, section 8): a good list signed by a
//! key is taken, and every other is refused. A throwaway key signs here, in
//! minisign's own format (prehashed Ed25519), as `tauri signer` writes the
//! project's real keys; the project's keys must refuse it.

mod support;

use support::{key, public_key, sign};
use zajil_android::tools::{check_android_tools, checked};
use zajil_core::tools_list::signed_by_any;

fn list(sequence: u64, version: &str, file: &str, sha256: &str) -> Vec<u8> {
    format!(
        r#"{{"format":"zajil-android-tools-1","sequence":{sequence},"issued_at":"2026-11-01T00:00:00Z","yt_dlp":{{"version":"{version}","file":"{file}","sha256":"{sha256}"}}}}"#
    )
    .into_bytes()
}

const SHA: &str = "e137c2171b0b8ef648e6136e86aa70c65923e93e434f01c93901d64744b190aa";

fn check_with_test_key(json: &[u8], signature: &str, newer_than: u64) -> Result<String, String> {
    let keys = [public_key(&key())];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    checked(json, signature, newer_than, |j, s| {
        signed_by_any(j, s, &keys)
    })
    .map(|tools| tools.yt_dlp_version)
    .map_err(|e| e.to_string())
}

#[test]
fn a_good_newer_list_is_taken() {
    let json = list(5, "2026.11.15", "yt-dlp", SHA);
    let signature = sign(&key(), &json);
    let keys = [public_key(&key())];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let tools = checked(&json, &signature, 4, |j, s| signed_by_any(j, s, &keys)).unwrap();
    assert_eq!(tools.sequence, 5);
    assert_eq!(tools.yt_dlp_version, "2026.11.15");
    assert_eq!(tools.yt_dlp_sha256, SHA);
}

#[test]
fn the_projects_keys_refuse_a_list_signed_by_another() {
    let json = list(5, "2026.11.15", "yt-dlp", SHA);
    let signature = sign(&key(), &json);
    let err = check_android_tools(json, signature, 0).unwrap_err();
    assert!(err.to_string().contains("not signed"), "{err}");
}

#[test]
fn a_list_changed_after_signing_is_refused() {
    let json = list(5, "2026.11.15", "yt-dlp", SHA);
    let signature = sign(&key(), &json);
    let changed = list(5, "2026.11.16", "yt-dlp", SHA);
    assert!(check_with_test_key(&changed, &signature, 0).is_err());
}

#[test]
fn a_list_no_newer_than_the_one_kept_is_refused() {
    let json = list(5, "2026.11.15", "yt-dlp", SHA);
    let signature = sign(&key(), &json);
    let err = check_with_test_key(&json, &signature, 5).unwrap_err();
    assert!(err.contains("not newer"), "{err}");
    assert!(check_with_test_key(&json, &signature, 9).is_err());
}

#[test]
fn odd_lists_are_refused_even_when_signed() {
    for (json, why) in [
        (
            list(5, "2026.11.15", "yt-dlp_linux", SHA),
            "Python release file",
        ),
        (list(5, "2026.11.15", "yt-dlp", "ABC"), "SHA-256"),
        (list(5, "../x", "yt-dlp", SHA), "plain name"),
        (
            b"{\"format\":\"zajil-tools-1\"}".to_vec(),
            "not a tools list",
        ),
    ] {
        let signature = sign(&key(), &json);
        let err = check_with_test_key(&json, &signature, 0).unwrap_err();
        assert!(err.contains(why), "{why}: {err}");
    }
}
