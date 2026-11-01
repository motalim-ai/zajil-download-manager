//! Each run's private folder, its record of the process tree, and its
//! cookie file; what a crash left is swept at the start.

use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use super::files::kept_folder;

/// The folders of runs, `<data dir>/run`.
pub struct Runs {
    pub(super) root: PathBuf,
}

pub(super) const RECORD_FILE: &str = "tree";
const OUT_FILE: &str = "out";
/// Held (locked) by a live run, so another Zajil never sweeps it.
const LOCK_FILE: &str = "lock";
pub(super) const COOKIE_FILE: &str = "cookies.txt";

impl Runs {
    pub fn new(root: impl Into<PathBuf>) -> Self {
        Runs { root: root.into() }
    }

    /// At program start, before any run: ends every process tree a crash
    /// left running, then removes the folders of those runs and their
    /// output folders. Never through a link (the runs folder must be a
    /// private folder of the user's), never a run another Zajil still runs
    /// (its lock is held), and past a run it cannot remove (the first error
    /// is returned at the end). Blocking.
    pub fn clean_leftovers(&self) -> io::Result<()> {
        match zajil_platform::fs::is_private_dir(&self.root) {
            Ok(true) => {}
            Ok(false) => {
                return Err(io::Error::new(
                    io::ErrorKind::PermissionDenied,
                    format!("{} is not a private folder", self.root.display()),
                ));
            }
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
            Err(err) => return Err(err),
        }
        let mut first_error = None;
        for entry in std::fs::read_dir(&self.root)? {
            let result = entry.and_then(|entry| clean_run(&entry.path()));
            if let Err(err) = result {
                first_error.get_or_insert(err);
            }
        }
        first_error.map_or(Ok(()), Err)
    }
}

/// One leftover run's folder `dir`, unless a live run holds its lock.
fn clean_run(dir: &Path) -> io::Result<()> {
    // Only a folder, never what a link points to.
    if !std::fs::symlink_metadata(dir)?.is_dir() {
        return Ok(());
    }
    match zajil_platform::fs::open_regular_nofollow(&dir.join(LOCK_FILE), true) {
        Ok(lock) => match lock.try_lock() {
            Ok(()) => {}
            // Another Zajil's live run (the app and `zajil`).
            Err(std::fs::TryLockError::WouldBlock) => return Ok(()),
            Err(std::fs::TryLockError::Error(err)) => return Err(err),
        },
        // A run that crashed before its lock existed.
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err),
    }
    if let Ok(line) = read_small(&dir.join(RECORD_FILE))
        && let Some(record) = zajil_platform::process::Record::from_line(&line)
    {
        zajil_platform::process::end_leftover(record)?;
    }
    if let Ok(out) = read_small(&dir.join(OUT_FILE)) {
        let out = PathBuf::from(out.trim_end_matches('\n'));
        // Only an output folder of ours: a `.zajil-video-*` folder.
        if out
            .file_name()
            .and_then(|n| n.to_str())
            .is_some_and(|n| n.starts_with(".zajil-video-"))
            && std::fs::symlink_metadata(&out).is_ok_and(|m| m.is_dir())
        {
            zajil_platform::fs::remove_dir_all_retrying(&out)?;
        }
    }
    zajil_platform::fs::remove_dir_all_retrying(dir)
}

/// A small text file, never through a link, at most 64 KiB.
fn read_small(path: &Path) -> io::Result<String> {
    let file = zajil_platform::fs::open_regular_nofollow(path, false)?;
    let mut text = String::new();
    file.take(64 * 1024).read_to_string(&mut text)?;
    Ok(text)
}

pub(super) fn random_hex() -> io::Result<String> {
    let mut bytes = [0u8; 8];
    getrandom::fill(&mut bytes).map_err(io::Error::other)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// One run's folders: its private folder under `<data dir>/run`, and, for
/// a download, its output folder next to the destination. Both are
/// removed by [`Run::close`] once the tree has ended. A `Run` is only ever
/// dropped without that inside [`Running`], which ends the tree first.
pub(super) struct Run {
    pub(super) dir: PathBuf,
    pub(super) out: Option<PathBuf>,
    /// The output folder is the video's own, kept by [`Run::close`] for
    /// the next run to continue its parts.
    pub(super) keep_out: bool,
    /// Locked while the run lives: another Zajil's sweep leaves it alone.
    lock: Option<std::fs::File>,
}

impl Run {
    pub(super) fn start(runs: &Runs, dest: Option<(&Path, Option<&str>)>) -> io::Result<Run> {
        zajil_platform::fs::private_dir(&runs.root)?;
        let dir = runs.root.join(random_hex()?);
        zajil_platform::fs::private_dir(&dir)?;
        let lock = zajil_platform::fs::create_private(&dir.join(LOCK_FILE))?;
        lock.lock()?;
        let mut run = Run {
            dir,
            out: None,
            keep_out: false,
            lock: Some(lock),
        };
        zajil_platform::fs::private_dir(&run.tmp())?;
        if let Some((dest, Some(key))) = dest {
            // The video's own folder, made or found again: a plain folder,
            // never a link. Not named in the run's record, so a sweep
            // after a crash keeps it too.
            let out = kept_folder(dest, key);
            match std::fs::create_dir(&out) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {
                    if !std::fs::symlink_metadata(&out)?.is_dir() {
                        return Err(io::Error::other(format!(
                            "{} is not a folder",
                            out.display()
                        )));
                    }
                }
                Err(err) => return Err(err),
            }
            run.out = Some(out);
            run.keep_out = true;
        } else if let Some((dest, None)) = dest {
            let out = dest.join(format!(".zajil-video-{}", random_hex()?));
            // Written before the folder exists, so a crash never leaves an
            // output folder that no record names.
            write_private(&run.dir.join(OUT_FILE), out.to_string_lossy().as_bytes())?;
            // A plain folder of its own (it holds no secret; review of the
            // video runner: a private one was refused on FAT32 and exFAT,
            // and made the video private): new, never one that is there or
            // a link.
            std::fs::create_dir(&out)?;
            run.out = Some(out);
        }
        Ok(run)
    }

    pub(super) fn tmp(&self) -> PathBuf {
        self.dir.join("tmp")
    }

    /// Where yt-dlp works: the output folder of a download, else the
    /// run's own folder.
    pub(super) fn work_dir(&self) -> PathBuf {
        self.out.clone().unwrap_or_else(|| self.dir.clone())
    }

    /// Removes the run's folders now, the cookie file first, again for a
    /// moment while an antivirus holds a file. Blocking; the tree has
    /// ended.
    pub(super) fn close(mut self) -> io::Result<()> {
        let (dir, out) = (std::mem::take(&mut self.dir), self.out.take());
        let out = out.filter(|_| !self.keep_out);
        // The lock is released before its folder goes (Windows cannot
        // remove an open file).
        drop(self.lock.take());
        let cookies = zajil_platform::fs::remove_file_retrying(&dir.join(COOKIE_FILE));
        let out = out.map_or(Ok(()), |out| {
            zajil_platform::fs::remove_dir_all_retrying(&out)
        });
        let dir = zajil_platform::fs::remove_dir_all_retrying(&dir);
        cookies.and(out).and(dir)
    }
}

impl Drop for Run {
    /// Only a run never closed: one whose start failed half-way, on its
    /// blocking thread, before any tree ran. A run with a tree is always
    /// closed, by [`Running`] if not before.
    fn drop(&mut self) {
        if !self.dir.as_os_str().is_empty() {
            let run = Run {
                dir: std::mem::take(&mut self.dir),
                out: self.out.take(),
                keep_out: self.keep_out,
                lock: self.lock.take(),
            };
            let _ = run.close();
        }
    }
}

/// A run and its tree, from the moment the tree starts. Dropped (a dropped
/// download future), it ends the tree, then removes the run's folders, in
/// that order and on a blocking thread (review of the video runner: the
/// folders could go while the tree still wrote, and the tree was ended on
/// an async thread).
pub(super) struct Running(pub(super) Option<(zajil_platform::process::Tree, Run)>);

impl Running {
    pub(super) fn take(mut self) -> (zajil_platform::process::Tree, Run) {
        self.0.take().expect("taken once")
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        let Some((tree, run)) = self.0.take() else {
            return;
        };
        let end = move || {
            drop(tree);
            let _ = run.close();
        };
        match tokio::runtime::Handle::try_current() {
            Ok(runtime) => drop(runtime.spawn_blocking(end)),
            Err(_) => end(),
        }
    }
}

pub(super) fn write_private(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let mut file = zajil_platform::fs::create_private(path)?;
    file.write_all(bytes)?;
    file.sync_all()
}

/// The cookies of one link, as the Netscape file yt-dlp reads: for the
/// link's host only (never its subdomains), sent only over HTTPS when the
/// link is HTTPS, kept for a day (yt-dlp drops cookies without expiry).
/// Cookies whose name or value could break a line are left out.
pub fn netscape_cookies(url: &reqwest::Url, header: &str, now_unix: u64) -> String {
    let host = url.host_str().unwrap_or_default();
    let secure = if url.scheme() == "https" {
        "TRUE"
    } else {
        "FALSE"
    };
    let expiry = now_unix.saturating_add(24 * 60 * 60);
    let mut out = String::from("# Netscape HTTP Cookie File\n");
    for pair in header.split(';') {
        let Some((name, value)) = pair.trim().split_once('=') else {
            continue;
        };
        let plain = |s: &str| !s.is_empty() && s.bytes().all(|b| b.is_ascii_graphic() && b != b';');
        if !plain(name) || !(value.is_empty() || plain(value)) {
            continue;
        }
        out.push_str(&format!(
            "{host}\tFALSE\t/\t{secure}\t{expiry}\t{name}\t{value}\n"
        ));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn cookies_go_to_the_link_s_host_only_and_nothing_breaks_a_line() {
        let url = reqwest::Url::parse("https://media.example.com/v.mp4?sig=x").unwrap();
        let text = netscape_cookies(
            &url,
            "sid=abc; theme=dark; bad\tname=1; x=y\nz; empty=",
            100,
        );
        assert_eq!(
            text,
            "# Netscape HTTP Cookie File\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\tsid\tabc\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\ttheme\tdark\n\
             media.example.com\tFALSE\t/\tTRUE\t86500\tempty\t\n"
        );
    }
}
