//! The Android protection lists file (ADR-0012, section 17): a good file
//! signed by a key is taken, and every other is refused. A throwaway key
//! signs here; the project's keys must refuse it.

mod support;

use support::{key, public_key, sign};
use zajil_android::lists::{check_android_lists, checked};
use zajil_core::tools_list::signed_by_any;

const SHA: &str = "e137c2171b0b8ef648e6136e86aa70c65923e93e434f01c93901d64744b190aa";

fn file(sequence: u64, lists: &str) -> Vec<u8> {
    format!(r#"{{"format":"zajil-android-lists-1","sequence":{sequence},"issued_at":"2026-11-01T00:00:00Z","lists":[{lists}]}}"#).into_bytes()
}

fn entry(name: &str, file: &str, size: u64) -> String {
    format!(r#"{{"name":"{name}","file":"{file}","sha256":"{SHA}","size":{size}}}"#)
}

fn check(json: &[u8], newer_than: u64) -> Result<Vec<String>, String> {
    let signature = sign(&key(), json);
    let keys = [public_key(&key())];
    let keys: Vec<&str> = keys.iter().map(String::as_str).collect();
    checked(json, &signature, newer_than, |j, s| {
        signed_by_any(j, s, &keys)
    })
    .map(|lists| lists.lists.into_iter().map(|l| l.file).collect())
    .map_err(|e| e.to_string())
}

#[test]
fn a_good_newer_file_is_taken() {
    let json = file(
        1_793_536_000,
        &[
            entry("ads", "zajil-lists-ads-1793536000.txt.gz", 396_537),
            entry("malware", "zajil-lists-malware-1793536000.txt.gz", 18_868),
        ]
        .join(","),
    );
    assert_eq!(
        check(&json, 1_793_000_000).unwrap(),
        [
            "zajil-lists-ads-1793536000.txt.gz",
            "zajil-lists-malware-1793536000.txt.gz"
        ]
    );
}

#[test]
fn the_projects_keys_refuse_a_file_signed_by_another() {
    let json = file(5, &entry("ads", "zajil-lists-ads-5.txt.gz", 10));
    let signature = sign(&key(), &json);
    assert!(check_android_lists(json, signature, 4).is_err());
}

#[test]
fn odd_files_are_refused_even_when_signed() {
    for (json, newer_than) in [
        // Not newer.
        (file(5, &entry("ads", "zajil-lists-ads-5.txt.gz", 10)), 5),
        // A list Zajil does not know, twice the same, none.
        (
            file(
                6,
                &entry("everything", "zajil-lists-everything-6.txt.gz", 10),
            ),
            5,
        ),
        (
            file(
                6,
                &[
                    entry("ads", "zajil-lists-ads-6.txt.gz", 10),
                    entry("ads", "zajil-lists-ads-7.txt.gz", 10),
                ]
                .join(","),
            ),
            5,
        ),
        (file(6, ""), 5),
        // A path, another list's name, or a link instead of a file name.
        (file(6, &entry("ads", "../zajil-lists-ads-6.txt.gz", 10)), 5),
        (
            file(6, &entry("ads", "zajil-lists-malware-6.txt.gz", 10)),
            5,
        ),
        (
            file(6, &entry("ads", "https://evil.example/ads.txt.gz", 10)),
            5,
        ),
        // Empty, or too big.
        (file(6, &entry("ads", "zajil-lists-ads-6.txt.gz", 0)), 5),
        (
            file(
                6,
                &entry("ads", "zajil-lists-ads-6.txt.gz", 9 * 1024 * 1024),
            ),
            5,
        ),
    ] {
        assert!(
            check(&json, newer_than).is_err(),
            "{}",
            String::from_utf8_lossy(&json)
        );
    }
    // A field it does not know: refused, not ignored.
    let extra = br#"{"format":"zajil-android-lists-1","sequence":6,"issued_at":"x","lists":[],"url":"https://evil.example"}"#;
    assert!(check(extra, 5).is_err());
}
