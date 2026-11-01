//! Zajil for Android's own update (ADR-0012, section 8; A4): a good signed
//! update file for a newer version is taken, and every other is refused.

mod support;

use support::{key, public_key, sign};
use zajil_android::update::{check_android_update, checked};
use zajil_core::tools_list::signed_by_any;

const SHA: &str = "4809bc7d6fb8044cead349b1ed54f0c18038aa1fcee27728bbd260a2882944ef";

fn update(version: &str, code: u64, apk: &str, sha256: &str, size: u64) -> Vec<u8> {
    format!(
        r#"{{"format":"zajil-android-update-1","version":"{version}","version_code":{code},"apk":"{apk}","sha256":"{sha256}","size":{size},"notes":"Faster videos.\nFixes."}}"#
    )
    .into_bytes()
}

fn good(version: &str, code: u64) -> Vec<u8> {
    update(
        version,
        code,
        &format!("zajil-android-{version}-arm64-v8a.apk"),
        SHA,
        25_000_000,
    )
}

fn check(json: &[u8], signature: &str, current: u64) -> Result<u64, String> {
    let keys = [public_key(&key())];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    checked(json, signature, current, |j, s| signed_by_any(j, s, &keys))
        .map(|u| u.version_code)
        .map_err(|e| e.to_string())
}

#[test]
fn a_good_newer_update_is_taken() {
    let json = good("0.2.0", 2000);
    let signature = sign(&key(), &json);
    let keys = [public_key(&key())];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    let u = checked(&json, &signature, 1000, |j, s| signed_by_any(j, s, &keys)).unwrap();
    assert_eq!(u.version, "0.2.0");
    assert_eq!(u.apk, "zajil-android-0.2.0-arm64-v8a.apk");
    assert_eq!(u.sha256, SHA);
    assert_eq!(u.notes, "Faster videos.\nFixes.");
}

#[test]
fn the_projects_keys_refuse_an_update_signed_by_another_key() {
    let json = good("0.2.0", 2000);
    let signature = sign(&key(), &json);
    let err = check_android_update(json, signature, 1000, None).unwrap_err();
    assert!(err.to_string().contains("not signed"), "{err}");
}

#[test]
fn a_debug_builds_test_key_is_taken_and_an_empty_one_is_none() {
    let json = good("0.2.0", 2000);
    let signature = sign(&key(), &json);
    let taken = check_android_update(
        json.clone(),
        signature.clone(),
        1000,
        Some(public_key(&key())),
    )
    .unwrap();
    assert_eq!(taken.version_code, 2000);
    // A release build's constant is empty: no key besides the project's.
    assert!(check_android_update(json, signature, 1000, Some(String::new())).is_err());
}

#[test]
fn the_same_or_an_older_version_is_refused() {
    let json = good("0.2.0", 2000);
    let signature = sign(&key(), &json);
    assert!(
        check(&json, &signature, 2000)
            .unwrap_err()
            .contains("not newer")
    );
    assert!(check(&json, &signature, 3000).is_err());
}

#[test]
fn an_update_changed_after_signing_is_refused() {
    let signature = sign(&key(), &good("0.2.0", 2000));
    assert!(check(&good("0.2.0", 2001), &signature, 1000).is_err());
}

#[test]
fn odd_updates_are_refused_even_when_signed() {
    for (json, why) in [
        (
            update("0.2.0", 2000, "zajil-android-0.3.0-arm64-v8a.apk", SHA, 1),
            "APK of that version",
        ),
        (
            update("0.2.0", 2000, "../evil.apk", SHA, 1),
            "APK of that version",
        ),
        (
            update("0.2", 2000, "zajil-android-0.2-arm64-v8a.apk", SHA, 1),
            "X.Y.Z",
        ),
        (
            update("0.02.0", 2000, "zajil-android-0.02.0-arm64-v8a.apk", SHA, 1),
            "X.Y.Z",
        ),
        (
            update("0.2.0", 2000, "zajil-android-0.2.0-arm64-v8a.apk", "XYZ", 1),
            "SHA-256",
        ),
        (
            update("0.2.0", 2000, "zajil-android-0.2.0-arm64-v8a.apk", SHA, 0),
            "size",
        ),
        (
            update(
                "0.2.0",
                2000,
                "zajil-android-0.2.0-arm64-v8a.apk",
                SHA,
                u64::MAX,
            ),
            "size",
        ),
        (
            b"{\"format\":\"zajil-android-tools-1\"}".to_vec(),
            "not an update file",
        ),
    ] {
        let signature = sign(&key(), &json);
        let err = check(&json, &signature, 1000).unwrap_err();
        assert!(err.contains(why), "{why}: {err}");
    }
}
