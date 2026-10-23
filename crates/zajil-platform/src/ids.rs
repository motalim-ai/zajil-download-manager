//! Fixed names and reverse-DNS identifiers (ADR-0001).
//!
//! These are the only place the identifiers are spelled out. Everything else
//! must use these constants.

/// Name of the per-user configuration, data and log folders.
pub const APP_DIR_NAME: &str = "zajil";

/// Reverse-DNS id with a hyphen. Used where Apple or Tauri require it:
/// `CFBundleIdentifier` on macOS and the Tauri `identifier` field.
pub const APPLE_BUNDLE_ID: &str = "io.github.motalim-ai.zajil";

/// Reverse-DNS id with an underscore. The default form: Linux app id
/// (Flatpak, AppStream, `.desktop`) and D-Bus names.
pub const APP_ID: &str = "io.github.motalim_ai.zajil";

/// Native Messaging host name for Chrome, Edge and Firefox on all systems.
/// On Windows it is also the registry key name.
pub const NATIVE_HOST_NAME: &str = "io.github.motalim_ai.zajil.host";

#[cfg(test)]
mod tests {
    use super::*;

    /// Apple and Tauri: ASCII letters, digits, hyphen and dot only.
    fn is_valid_bundle_id(id: &str) -> bool {
        !id.is_empty()
            && id
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'.')
    }

    /// Chrome: lowercase letters, digits, underscore and dot; no leading,
    /// trailing or doubled dot. Firefox: `^\w+(\.\w+)*$`. Both must hold.
    fn is_valid_native_host_name(name: &str) -> bool {
        name.split('.').all(|part| {
            !part.is_empty()
                && part
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
        })
    }

    /// D-Bus / Flatpak: elements of `[A-Za-z0-9_]` that do not start with a
    /// digit, at least three elements.
    fn is_valid_dbus_app_id(id: &str) -> bool {
        let parts: Vec<&str> = id.split('.').collect();
        parts.len() >= 3
            && parts.iter().all(|part| {
                !part.is_empty()
                    && !part.as_bytes()[0].is_ascii_digit()
                    && part.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'_')
            })
    }

    #[test]
    fn bundle_id_is_valid_for_apple_and_tauri() {
        assert!(is_valid_bundle_id(APPLE_BUNDLE_ID));
    }

    #[test]
    fn native_host_name_is_valid_for_chrome_and_firefox() {
        assert!(is_valid_native_host_name(NATIVE_HOST_NAME));
        assert!(!is_valid_native_host_name(APPLE_BUNDLE_ID));
    }

    #[test]
    fn app_id_is_valid_for_dbus() {
        assert!(is_valid_dbus_app_id(APP_ID));
        assert!(!is_valid_dbus_app_id(APPLE_BUNDLE_ID));
    }

    #[test]
    fn the_two_forms_differ_only_in_the_separator() {
        assert_eq!(APPLE_BUNDLE_ID.replace('-', "_"), APP_ID);
        assert_eq!(format!("{APP_ID}.host"), NATIVE_HOST_NAME);
    }
}
