use std::fs;
use std::time::{Duration, Instant};

use super::*;

/// Owner decision (M2): a sparse file on Windows, so writes far from the
/// start do not fill the gap with zeros first.
#[test]
fn allocate_makes_a_sparse_file_on_windows() {
    use std::os::windows::fs::MetadataExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_ATTRIBUTE_SPARSE_FILE;
    let dir = std::env::temp_dir().join(format!("zajil-sparse-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = fs::File::create(dir.join("part")).unwrap();
    allocate(&file, 64 << 20).unwrap();
    let meta = file.metadata().unwrap();
    assert_eq!(meta.len(), 64 << 20);
    assert_ne!(meta.file_attributes() & FILE_ATTRIBUTE_SPARSE_FILE, 0);
    drop(file);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn replace_file_waits_for_a_scanner_to_let_go() {
    let dir = std::env::temp_dir().join(format!("zajil-retry-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let (tmp, state) = (dir.join("x.zajil-state.tmp"), dir.join("x.zajil-state"));
    fs::write(&tmp, b"new").unwrap();
    fs::write(&state, b"old").unwrap();

    // Read access without FILE_SHARE_DELETE, like a scanner.
    let held = open_like_a_scanner(&state).unwrap();
    // Without the retry the rename fails right away.
    assert!(imp::replace_file(&tmp, &state).is_err());

    let release = std::thread::spawn(move || {
        std::thread::sleep(Duration::from_millis(150));
        drop(held);
    });
    let started = Instant::now();
    replace_file(&tmp, &state).unwrap();
    assert!(started.elapsed() >= Duration::from_millis(100));
    assert_eq!(fs::read(&state).unwrap(), b"new");
    release.join().unwrap();
    let _ = fs::remove_dir_all(&dir);
}
