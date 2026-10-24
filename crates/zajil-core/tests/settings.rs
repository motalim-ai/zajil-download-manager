//! The desktop's settings (M4): `settings.json`, by the same rules as the
//! categories and queues files (ADR-0007).

use std::fs;

use zajil_core::settings::{Digits, Language, SettingsError, SettingsStore, Theme};
use zajil_core::state::checksum_of;

/// What a damaged or hostile settings file must never do: be used, or be
/// written over.
fn assert_refused_and_kept(store: &SettingsStore, expect: &str) {
    let before = fs::read(store.path()).ok();
    let err = store.load().unwrap_err();
    assert!(err.to_string().contains(expect), "{err}");
    let change = store.update(|file| {
        file.connections = 3;
        Ok(())
    });
    assert!(change.is_err());
    assert_eq!(fs::read(store.path()).ok(), before, "the file was changed");
}

#[test]
fn without_a_file_the_defaults_are_arabic_the_system_theme_and_123() {
    let data = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(data.path());
    let file = store.load().unwrap();
    assert_eq!(file.language, Language::Ar);
    assert_eq!(file.theme, Theme::System);
    assert_eq!(file.digits, Digits::Latn);
    assert_eq!(file.connections, 8);
    assert!(file.download_dir.is_none());
    assert!(!store.path().exists());
}

#[test]
fn a_change_is_kept_in_a_file_only_its_owner_can_read() {
    let data = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(data.path());
    let downloads = data.path().join("تحميلاتي");
    store
        .update(|file| {
            file.language = Language::En;
            file.theme = Theme::Dark;
            file.digits = Digits::Arab;
            file.connections = 16;
            file.download_dir = Some(downloads.clone());
            Ok(())
        })
        .unwrap();
    // A new store reads it from disk, as after a restart.
    let again = SettingsStore::new(data.path()).load().unwrap();
    assert_eq!(again.language, Language::En);
    assert_eq!(again.theme, Theme::Dark);
    assert_eq!(again.digits, Digits::Arab);
    assert_eq!(again.connections, 16);
    assert_eq!(again.download_dir.as_deref(), Some(downloads.as_path()));
    assert!(zajil_platform::fs::only_owner_can_access(&store.path()).unwrap());
    let text = fs::read_to_string(store.path()).unwrap();
    assert!(text.contains("\"format\": \"zajil-settings\""), "{text}");
    assert!(text.contains("\"schema_version\": 1"), "{text}");
}

#[test]
fn values_out_of_range_are_never_written() {
    let data = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(data.path());
    for bad in [0, 33, u32::MAX] {
        let err = store
            .update(|file| {
                file.connections = bad;
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(_)), "{err}");
    }
    for bad in ["relative/dir", "/a/../b", "/a/\u{202e}b", "/a/\u{3164}b"] {
        let err = store
            .update(|file| {
                file.download_dir = Some(bad.into());
                Ok(())
            })
            .unwrap_err();
        assert!(matches!(err, SettingsError::Invalid(_)), "{bad}: {err}");
    }
    assert!(!store.path().exists());
}

#[test]
fn a_damaged_file_is_refused_and_never_written_over() {
    let data = tempfile::tempdir().unwrap();
    let store = SettingsStore::new(data.path());
    store
        .update(|file| {
            file.connections = 4;
            Ok(())
        })
        .unwrap();
    let good = fs::read_to_string(store.path()).unwrap();

    fs::write(
        store.path(),
        good.replace("\"connections\": 4", "\"connections\": 5"),
    )
    .unwrap();
    assert_refused_and_kept(&store, "checksum does not match");

    fs::write(store.path(), &good[..good.len() / 2]).unwrap();
    assert_refused_and_kept(&store, "damaged");

    let signed = |change: &dyn Fn(&mut serde_json::Value)| {
        let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
        change(&mut value);
        value["checksum"] = checksum_of(&value).into();
        fs::write(store.path(), value.to_string()).unwrap();
    };
    signed(&|v| v["run"] = "rm -rf /".into());
    assert_refused_and_kept(&store, "unknown field");
    signed(&|v| v["language"] = "fr".into());
    assert_refused_and_kept(&store, "unknown variant");
    signed(&|v| v["connections"] = 1000.into());
    assert_refused_and_kept(&store, "connections must be from 1 to 32");
    signed(&|v| v["download_dir"] = "../../etc".into());
    assert_refused_and_kept(&store, "invalid folder");

    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["schema_version"] = 99.into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "schema version 99");

    let mut value: serde_json::Value = serde_json::from_str(&good).unwrap();
    value["format"] = "zajil-queues".into();
    fs::write(store.path(), value.to_string()).unwrap();
    assert_refused_and_kept(&store, "not a Zajil settings file");

    fs::write(store.path(), vec![b' '; 64 * 1024 + 1]).unwrap();
    assert_refused_and_kept(&store, "too large");
}

#[test]
fn a_link_in_place_of_the_file_is_refused_and_what_it_points_to_is_untouched() {
    let data = tempfile::tempdir().unwrap();
    let outside = tempfile::tempdir().unwrap();
    let elsewhere = SettingsStore::new(outside.path());
    elsewhere
        .update(|file| {
            file.connections = 2;
            Ok(())
        })
        .unwrap();
    let target_before = fs::read(elsewhere.path()).unwrap();
    let store = SettingsStore::new(data.path());
    zajil_platform::fs::symlink_file(&elsewhere.path(), &store.path())
        .expect("this test must be able to create a symbolic link");
    assert!(matches!(
        store.load(),
        Err(SettingsError::NotRegularFile(_))
    ));
    assert!(
        store
            .update(|file| {
                file.connections = 9;
                Ok(())
            })
            .is_err()
    );
    assert_eq!(fs::read(elsewhere.path()).unwrap(), target_before);
}
