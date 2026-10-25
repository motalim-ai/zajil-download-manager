//! `fs::private_dir` makes every level it creates private, whatever the
//! umask (review of the video tools: `create_dir_all` left the parents
//! group-writable under umask 002, the default of desktop Linux, and every
//! later check refused them). One test in this program: the umask is the
//! whole process's.

use zajil_platform::fs::{is_private_dir, private_dir};

#[test]
fn every_level_private_dir_creates_is_private() {
    #[cfg(unix)]
    // SAFETY: umask has no preconditions; this program has no other test,
    // so no other thread creates files meanwhile.
    #[allow(unsafe_code)]
    unsafe {
        libc::umask(0o002);
    }
    let root = tempfile::tempdir().unwrap();
    let deep = root
        .path()
        .join("data")
        .join("tools")
        .join("yt-dlp")
        .join("2099.01.01");
    private_dir(&deep).unwrap();
    for level in [
        root.path().join("data"),
        root.path().join("data").join("tools"),
        root.path().join("data").join("tools").join("yt-dlp"),
        deep.clone(),
    ] {
        assert!(is_private_dir(&level).unwrap(), "{}", level.display());
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            let mode = std::fs::metadata(&level).unwrap().permissions().mode() & 0o777;
            assert_eq!(mode, 0o700, "{}", level.display());
        }
    }
    // Asking again for an existing private folder is fine.
    private_dir(&deep).unwrap();
}
