use std::fs;

use super::*;

/// Review #84: on Windows the check was `true` whatever the
/// permissions. A folder of the user's own is trusted, and a program in
/// it; once others may change the folder (as under `C:\`, where
/// Authenticated Users may modify), or only the program, neither is;
/// letting others read is fine.
#[cfg(windows)]
#[test]
fn others_who_may_change_a_program_or_its_folder_are_seen_on_windows() {
    let icacls = |path: &Path, grant: &str| {
        let status = std::process::Command::new("icacls")
            .arg(path)
            .args(["/grant", grant])
            .stdout(std::process::Stdio::null())
            .status()
            .unwrap();
        assert!(status.success(), "icacls {grant}");
    };
    let mine = tempdir();
    let program = mine.as_path().join("zajil-host.exe");
    fs::write(&program, b"MZ").unwrap();
    assert!(only_user_or_root_can_change(mine.as_path()).unwrap());
    assert!(only_user_or_root_can_change(&program).unwrap());

    // Everyone may read and run: still only the user can change it.
    icacls(&program, "*S-1-1-0:(RX)");
    assert!(only_user_or_root_can_change(&program).unwrap());

    // Everyone may write to the program alone.
    icacls(&program, "*S-1-1-0:(W)");
    assert!(!only_user_or_root_can_change(&program).unwrap());

    // Authenticated Users may modify the folder, and what is made in it.
    let shared = tempdir();
    icacls(shared.as_path(), "*S-1-5-11:(OI)(CI)(M)");
    let program = shared.as_path().join("zajil-host.exe");
    fs::write(&program, b"MZ").unwrap();
    assert!(!only_user_or_root_can_change(shared.as_path()).unwrap());
    assert!(!only_user_or_root_can_change(&program).unwrap());
    let _ = fs::remove_dir_all(&mine);
    let _ = fs::remove_dir_all(&shared);
}

/// The same on macOS and Linux: writable by others, or not.
#[cfg(unix)]
#[test]
fn others_who_may_change_a_program_or_its_folder_are_seen_on_unix() {
    use std::os::unix::fs::PermissionsExt;
    let dir = tempdir();
    let program = dir.as_path().join("zajil-host");
    fs::write(&program, b"#!").unwrap();
    fs::set_permissions(&program, fs::Permissions::from_mode(0o755)).unwrap();
    fs::set_permissions(dir.as_path(), fs::Permissions::from_mode(0o755)).unwrap();
    assert!(only_user_or_root_can_change(dir.as_path()).unwrap());
    assert!(only_user_or_root_can_change(&program).unwrap());
    fs::set_permissions(&program, fs::Permissions::from_mode(0o757)).unwrap();
    assert!(!only_user_or_root_can_change(&program).unwrap());
    fs::set_permissions(dir.as_path(), fs::Permissions::from_mode(0o775)).unwrap();
    assert!(!only_user_or_root_can_change(dir.as_path()).unwrap());
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn syncs_an_existing_folder() {
    sync_dir(&std::env::temp_dir()).unwrap();
}

#[test]
fn reports_a_missing_folder() {
    let missing = std::env::temp_dir().join("zajil-no-such-folder-for-sync");
    assert!(sync_dir(&missing).is_err());
}

#[test]
fn replace_file_replaces() {
    let dir = tempdir();
    let (a, b) = (dir.join("a"), dir.join("b"));
    fs::write(&a, b"new").unwrap();
    fs::write(&b, b"old").unwrap();
    replace_file(&a, &b).unwrap();
    assert_eq!(fs::read(&b).unwrap(), b"new");
    assert!(!a.exists());
}

/// #55: the name is free at once, even while a handle on the file is
/// open (as an antivirus holds it, sharing its deletion), and nothing
/// is ever replaced.
#[test]
fn move_aside_frees_the_name_while_the_file_is_held() {
    let dir = tempdir();
    let path = dir.join("f.bin.zajil-part");
    std::fs::write(&path, b"held").unwrap();
    let held = File::open(&path).unwrap();
    let aside = move_aside(&path).unwrap();
    assert_eq!(aside.parent(), Some(dir.as_path()));
    assert!(
        aside
            .file_name()
            .unwrap()
            .to_string_lossy()
            .starts_with(".zajil-removed-")
    );
    // The name is free for a new file at once.
    let new = std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&path);
    assert!(new.is_ok(), "{new:?}");
    assert_eq!(std::fs::read(&aside).unwrap(), b"held");
    std::fs::remove_file(&aside).unwrap();
    drop(held);
    // A missing file is NotFound; a second move takes another name.
    assert_eq!(
        move_aside(&dir.join("none")).unwrap_err().kind(),
        io::ErrorKind::NotFound
    );
    let other = move_aside(&path).unwrap();
    assert_ne!(other, aside);
}

#[test]
fn rename_noreplace_never_overwrites() {
    let dir = tempdir();
    let (a, b) = (dir.join("a"), dir.join("b"));
    fs::write(&a, b"new").unwrap();
    fs::write(&b, b"old").unwrap();
    let err = rename_noreplace(&a, &b).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&b).unwrap(), b"old");
    assert_eq!(fs::read(&a).unwrap(), b"new");

    let c = dir.join("c");
    rename_noreplace(&a, &c).unwrap();
    assert_eq!(fs::read(&c).unwrap(), b"new");
    assert!(!a.exists());
}

#[test]
fn opens_a_regular_file() {
    let dir = tempdir();
    let path = dir.join("f");
    fs::write(&path, b"x").unwrap();
    open_regular_nofollow(&path, true).unwrap();
    open_regular_nofollow(&path, false).unwrap();
}

#[test]
fn refuses_a_symbolic_link_and_leaves_its_target_alone() {
    let dir = tempdir();
    let target = dir.join("victim");
    fs::write(&target, b"precious").unwrap();
    let link = dir.join("link");
    if !symlinks_available(&target, &link) {
        return;
    }
    let err = open_regular_nofollow(&link, true).unwrap_err();
    assert!(is_not_regular_file(&err), "{err}");
    assert_eq!(fs::read(&target).unwrap(), b"precious");
}

#[test]
fn refuses_a_hard_link() {
    let dir = tempdir();
    let target = dir.join("victim");
    fs::write(&target, b"precious").unwrap();
    let link = dir.join("hard");
    fs::hard_link(&target, &link).unwrap();
    let err = open_regular_nofollow(&link, true).unwrap_err();
    assert!(is_not_regular_file(&err), "{err}");
}

#[test]
fn refuses_a_folder() {
    let dir = tempdir();
    assert!(open_regular_nofollow(&dir, false).is_err());
}

#[test]
fn write_at_fills_pieces_in_any_order_through_the_locked_handle() {
    let dir = tempdir();
    let path = dir.join("part");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    // The download's lock: on Windows it belongs to this handle, so the
    // writes must go through it.
    file.try_lock().unwrap();
    allocate(&file, 12).unwrap();
    write_at(&file, b"89ab", 8).unwrap();
    write_at(&file, b"0123", 0).unwrap();
    write_at(&file, b"4567", 4).unwrap();
    file.sync_all().unwrap();
    // While the lock is held, Windows refuses even reads through another
    // handle: the reason every connection writes through this one.
    if cfg!(windows) {
        assert!(fs::read(&path).is_err());
    }
    drop(file);
    assert_eq!(fs::read(&path).unwrap(), b"0123456789ab");
}

#[test]
fn allocate_gives_the_length_and_reads_back_zeros() {
    let dir = tempdir();
    let path = dir.join("part");
    let file = fs::File::create(&path).unwrap();
    allocate(&file, 1 << 20).unwrap();
    drop(file);
    let data = fs::read(&path).unwrap();
    assert_eq!(data.len(), 1 << 20);
    assert!(data.iter().all(|&b| b == 0));
}

#[test]
fn allocate_refuses_a_file_that_is_not_empty() {
    let dir = tempdir();
    let path = dir.join("part");
    fs::write(&path, b"data").unwrap();
    let file = fs::OpenOptions::new().write(true).open(&path).unwrap();
    let err = allocate(&file, 10).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
    assert_eq!(fs::read(&path).unwrap(), b"data");
}

#[test]
fn allocate_refuses_absurd_sizes_without_panicking() {
    let dir = tempdir();
    let file = fs::File::create(dir.join("part")).unwrap();
    for len in [u64::MAX, i64::MAX as u64 + 1] {
        let err = allocate(&file, len).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::FileTooLarge, "{len}");
    }
    // Valid as a number: APFS, ext4 and NTFS refuse a file that large; a
    // file system that allows it makes a sparse file of it. Either way,
    // no panic and nothing half done.
    match allocate(&file, i64::MAX as u64) {
        Err(_) => assert_eq!(file.metadata().unwrap().len(), 0),
        Ok(()) => assert_eq!(file.metadata().unwrap().len(), i64::MAX as u64),
    }
}

#[test]
fn allocate_reserves_nothing_and_far_writes_take_only_their_own_blocks() {
    const GIB: u64 = 1 << 30;
    const PIECE: usize = 64 << 10;
    let dir = tempdir();
    let path = dir.join("part");
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .create_new(true)
        .open(&path)
        .unwrap();
    allocate(&file, GIB).unwrap();
    file.sync_all().unwrap();
    let empty = size_on_disk(&path).unwrap();
    eprintln!("1 GiB right after allocate: {empty} bytes on disk");
    assert_eq!(file.metadata().unwrap().len(), GIB);
    assert!(empty < 1 << 20, "{empty} bytes reserved");

    // Pieces far apart, as the connections of a split download write
    // them: the gaps stay holes, never zeros written to the disk.
    let offsets = [0, GIB / 4, GIB / 2, GIB - PIECE as u64];
    for (i, &at) in offsets.iter().enumerate() {
        write_at(&file, &[i as u8 + 1; PIECE], at).unwrap();
    }
    file.sync_all().unwrap();
    let used = size_on_disk(&path).unwrap();
    eprintln!("after 4 pieces of 64 KiB: {used} bytes on disk");
    assert!(used >= 4 * PIECE as u64 && used < 4 << 20, "{used} bytes");
    drop(file);

    let file = fs::File::open(&path).unwrap();
    for (i, &at) in offsets.iter().enumerate() {
        let mut piece = vec![0; PIECE];
        read_exact_at(&file, &mut piece, at);
        assert!(piece.iter().all(|&b| b == i as u8 + 1), "piece {i}");
    }
    let mut gap = vec![1; PIECE];
    read_exact_at(&file, &mut gap, GIB / 8);
    assert!(gap.iter().all(|&b| b == 0));
}

fn read_exact_at(file: &fs::File, buf: &mut [u8], at: u64) {
    use std::io::{Read, Seek, SeekFrom};
    let mut file = file;
    file.seek(SeekFrom::Start(at)).unwrap();
    file.read_exact(buf).unwrap();
}

#[test]
fn allocate_of_zero_is_an_empty_file() {
    let dir = tempdir();
    let file = fs::File::create(dir.join("part")).unwrap();
    allocate(&file, 0).unwrap();
    assert_eq!(file.metadata().unwrap().len(), 0);
}

#[test]
fn reports_available_space() {
    let dir = tempdir();
    let free = available_space(&dir).unwrap();
    assert!(free > 0);
    assert!(available_space(&dir.join("missing")).is_err());
}

#[test]
fn check_then_rename_refuses_anything_at_the_target() {
    let dir = tempdir();
    let (a, b) = (dir.join("a"), dir.join("b"));
    fs::write(&a, b"new").unwrap();
    fs::write(&b, b"old").unwrap();
    let err = check_then_rename(&a, &b).unwrap_err();
    assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
    assert_eq!(fs::read(&a).unwrap(), b"new");
    assert_eq!(fs::read(&b).unwrap(), b"old");

    // A dangling symbolic link counts as taken, and is left alone.
    let link = dir.join("link");
    if symlinks_available(&dir.join("nowhere"), &link) {
        let err = check_then_rename(&a, &link).unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::AlreadyExists);
        assert!(link.symlink_metadata().unwrap().file_type().is_symlink());
        assert!(!dir.join("nowhere").exists());
    }

    let c = dir.join("c");
    check_then_rename(&a, &c).unwrap();
    assert_eq!(fs::read(&c).unwrap(), b"new");
    assert!(!a.exists());
}

#[cfg(unix)]
#[test]
fn only_link_errors_that_mean_no_hard_links_choose_the_last_resort() {
    for code in [libc::EPERM, libc::ENOTSUP, libc::EOPNOTSUPP] {
        assert!(imp::links_unsupported(&io::Error::from_raw_os_error(code)));
    }
    for code in [libc::EEXIST, libc::EACCES, libc::ENOSPC, libc::EXDEV] {
        assert!(!imp::links_unsupported(&io::Error::from_raw_os_error(code)));
    }
}

/// Creates a symbolic link, or reports that this machine cannot. On CI
/// (where `CI` is set) that is a failure, so the test cannot silently
/// stop testing anything.
fn symlinks_available(target: &Path, link: &Path) -> bool {
    match symlink_file(target, link) {
        Ok(()) => true,
        Err(err) if std::env::var_os("CI").is_none() => {
            eprintln!("skipping: cannot create symbolic links here: {err}");
            false
        }
        Err(err) => panic!("CI must be able to create symbolic links: {err}"),
    }
}

#[test]
fn private_files_are_for_the_current_user_only() {
    use std::io::Write;
    let dir = tempdir();
    let path = dir.join("state");
    let mut file = create_private(&path).unwrap();
    file.write_all(b"?sig=SECRET").unwrap();
    drop(file);
    assert_eq!(fs::read(&path).unwrap(), b"?sig=SECRET");
    assert!(only_owner_can_access(&path).unwrap());
    // Like `create_new`: never opens what is already there.
    assert_eq!(
        create_private(&path).unwrap_err().kind(),
        io::ErrorKind::AlreadyExists
    );
    // Replacing another file (as a state save does) keeps it private.
    let old = dir.join("old");
    fs::write(&old, b"old").unwrap();
    replace_file(&path, &old).unwrap();
    assert!(only_owner_can_access(&old).unwrap());
    // Control: a file made the ordinary way is readable by others, so
    // the check can say no.
    let plain = dir.join("plain");
    fs::write(&plain, b"x").unwrap();
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o644)).unwrap();
    }
    assert!(!only_owner_can_access(&plain).unwrap());
    check_with_system_tool(&old);
    let _ = fs::remove_dir_all(&dir);
}

/// Review of #30: on macOS a folder's inheritable access list made a
/// `0600` state file readable by everyone, and the check said it was
/// private. The test gives the folder such a list with `chmod +a`, the
/// way the reviewer did, and reads the result with `ls -le`.
#[cfg(target_os = "macos")]
#[test]
fn an_inherited_access_list_is_removed_on_macos() {
    use std::process::Command;
    let dir = tempdir();
    let status = Command::new("/bin/chmod")
        .args(["+a", "everyone allow read,file_inherit"])
        .arg(&dir)
        .status()
        .unwrap();
    assert!(status.success());
    // Control: a file made the ordinary way inherits it, and the check
    // sees it even with mode 0600.
    let plain = dir.join("plain");
    fs::write(&plain, b"x").unwrap();
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(&plain, fs::Permissions::from_mode(0o600)).unwrap();
    }
    let listing = |path: &Path| {
        let out = Command::new("/bin/ls")
            .arg("-le")
            .arg(path)
            .output()
            .unwrap();
        String::from_utf8_lossy(&out.stdout).into_owned()
    };
    assert!(listing(&plain).contains("everyone"), "{}", listing(&plain));
    assert!(!only_owner_can_access(&plain).unwrap());

    let private = dir.join("state");
    drop(create_private(&private).unwrap());
    assert!(
        !listing(&private).contains("everyone"),
        "{}",
        listing(&private)
    );
    assert!(only_owner_can_access(&private).unwrap());
    // Still private after it replaces another file, as a save does.
    let old = dir.join("old");
    fs::write(&old, b"old").unwrap();
    replace_file(&private, &old).unwrap();
    assert!(!listing(&old).contains("everyone"), "{}", listing(&old));
    assert!(only_owner_can_access(&old).unwrap());
    let _ = fs::remove_dir_all(&dir);
}

/// Reads the permissions of `path` without our own code: the mode from
/// the standard library on macOS and Linux, and `icacls` on Windows,
/// which must list one entry, full access for `whoami`, not inherited.
#[cfg(unix)]
fn check_with_system_tool(path: &Path) {
    use std::os::unix::fs::PermissionsExt;
    let mode = fs::metadata(path).unwrap().permissions().mode() & 0o777;
    assert_eq!(mode, 0o600, "{mode:o}");
}

#[cfg(windows)]
fn check_with_system_tool(path: &Path) {
    use std::process::Command;
    let run = |command: &mut Command| {
        let out = command.output().unwrap();
        assert!(out.status.success(), "{out:?}");
        String::from_utf8_lossy(&out.stdout).to_lowercase()
    };
    let user = run(&mut Command::new("whoami")).trim().to_owned();
    let listing = run(Command::new("icacls").arg(path));
    let entries: Vec<&str> = listing.lines().filter(|l| l.contains(":(")).collect();
    assert_eq!(entries.len(), 1, "{listing}");
    assert!(
        entries[0].contains(&format!("{user}:(f)")),
        "{user}: {listing}"
    );
    assert!(!listing.contains("(i)"), "inherited entry: {listing}");
}

fn tempdir() -> PathBuf {
    use std::sync::atomic::{AtomicU32, Ordering};
    static N: AtomicU32 = AtomicU32::new(0);
    let dir = std::env::temp_dir().join(format!(
        "zajil-platform-fs-{}-{}",
        std::process::id(),
        N.fetch_add(1, Ordering::Relaxed)
    ));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
