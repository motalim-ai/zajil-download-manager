//! Registering `zajil-host` with each browser, for the current user only
//! (`NativeHostRegistrar`, ADR-0006; ADR-0005). A browser starts a Native
//! Messaging host only if a manifest names it and lists the extension:
//! `allowed_origins` (`chrome-extension://<id>/`) in Chrome, Chromium and
//! Edge, `allowed_extensions` (the add-on's id) in Firefox. The browser
//! refuses every other extension; that is the protection (ADR-0005).
//!
//! | | Manifest | Found by the browser through |
//! |---|---|---|
//! | macOS | `~/Library/Application Support/<browser>/NativeMessagingHosts/<name>.json` | its folder |
//! | Linux | `<config>/<browser>/NativeMessagingHosts/<name>.json`, Firefox `~/.mozilla/native-messaging-hosts/<name>.json` | its folder |
//! | Windows | `<data dir>/native-messaging/<browser>.json` | the default value of `HKCU\Software\<vendor>\NativeMessagingHosts\<name>` |
//!
//! `<config>` on Linux is `$CHROME_CONFIG_HOME`, else `$XDG_CONFIG_HOME`,
//! else `~/.config`, as Chromium reads it. Firefox keeps `~/.mozilla` for
//! manifests even when its profiles are under `~/.config/mozilla`.
//!
//! Every supported browser is registered, installed or not: a browser's
//! folder or key appears only once it has run, and a browser installed
//! later then finds Zajil's host at once. Registering records each folder
//! and key it creates (`<data dir>/native-messaging/<browser>.made`), and
//! makes folders readable by the user only, as the browsers make theirs.
//! Unregistering removes the manifest and Zajil's registry key, then each
//! recorded folder or key that is still empty: what was there before
//! registering stays, even empty, and so does what the browser has put in
//! since. A record line naming anything but one of these folders or keys
//! is ignored.
//!
//! Not covered: Flatpak browsers and the Chromium snap run in a sandbox
//! that does not start a program outside it; nothing is registered inside
//! them. A browser from the Microsoft Store may keep its own copy of
//! `HKCU`; not checked yet.
//!
//! Extension ids are checked before they go into a manifest the browser
//! trusts: 32 letters `a`–`p` for Chrome-family ids; `name@domain` or a
//! `{uuid}` for Firefox.

use std::io::{self, Write};
use std::path::{Path, PathBuf};

use crate::ids::NATIVE_HOST_NAME;

/// The browsers Zajil's extension supports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Browser {
    Chrome,
    Chromium,
    Edge,
    Firefox,
}

impl Browser {
    pub const ALL: [Browser; 4] = [
        Browser::Chrome,
        Browser::Chromium,
        Browser::Edge,
        Browser::Firefox,
    ];

    /// Firefox lists add-on ids; the others list `chrome-extension://` origins.
    fn is_firefox(self) -> bool {
        self == Browser::Firefox
    }

    /// The vendor key under `HKCU\<root>` (Windows); `NativeMessagingHosts`
    /// goes under it.
    #[cfg(windows)]
    fn vendor_key(self) -> &'static str {
        match self {
            Browser::Chrome => r"Google\Chrome",
            Browser::Chromium => "Chromium",
            Browser::Edge => r"Microsoft\Edge",
            Browser::Firefox => "Mozilla",
        }
    }

    fn file_stem(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Chromium => "chromium",
            Browser::Edge => "edge",
            Browser::Firefox => "firefox",
        }
    }
}

/// The folder in Zajil's data folder that holds the copy of `zajil-host`
/// browsers start when Zajil runs from a place that goes away when it
/// closes ([`fixed_copy`]).
pub const FIXED_HOST_DIR: &str = "bin";

/// Copies the host program `host` to `dir`, under its own file name, for
/// browsers to start when `host` sits in a place that goes away when Zajil
/// closes: an AppImage's mount (#129). Returns the copy's path.
///
/// `dir` is made a folder only this user can change
/// ([`crate::fs::private_dir`]). A copy already there with the same SHA-256
/// is kept, so the copy follows the AppImage when it is updated. Anything
/// else there (another version, a link, a second name of another file) is
/// replaced whole: the copy is written under a new name, made runnable by
/// its owner only, and renamed over it, so nothing is written through a
/// link. The copy is then opened as a program Zajil checks
/// ([`crate::fs::open_private_tool`]: a regular file with one name, never a
/// link, that only this user or root can change) and its SHA-256 checked
/// again.
pub fn fixed_copy(host: &Path, dir: &Path) -> io::Result<PathBuf> {
    let name = host
        .file_name()
        .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "the host has no file name"))?;
    let program = std::fs::read(host)?;
    let digest = sha256(&program);
    crate::fs::private_dir(dir)?;
    let copy = dir.join(name);
    if copy_digest(&copy).ok() == Some(digest) {
        return Ok(copy);
    }
    let mut random = [0u8; 8];
    getrandom::fill(&mut random).map_err(io::Error::other)?;
    let suffix: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let new = dir.join(format!(".{}.{suffix}.new", name.to_string_lossy()));
    let written = (|| {
        let mut file = crate::fs::create_private(&new)?;
        file.write_all(&program)?;
        crate::fs::make_executable(&file)?;
        file.sync_all()?;
        drop(file);
        crate::fs::replace_file(&new, &copy)?;
        crate::fs::sync_dir(dir)
    })();
    if written.is_err() {
        let _ = std::fs::remove_file(&new);
    }
    written?;
    if copy_digest(&copy)? != digest {
        return Err(io::Error::other(format!(
            "{} is not the same as {}",
            copy.display(),
            host.display()
        )));
    }
    Ok(copy)
}

/// The SHA-256 of the program at `path`, read as a program Zajil checks
/// ([`crate::fs::open_private_tool`]); on macOS and Linux it must be one
/// its owner can run.
fn copy_digest(path: &Path) -> io::Result<[u8; 32]> {
    use std::io::Read;
    let mut file = crate::fs::open_private_tool(path)?;
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        if file.metadata()?.permissions().mode() & 0o100 == 0 {
            return Err(io::Error::new(
                io::ErrorKind::PermissionDenied,
                format!("{} cannot be run", path.display()),
            ));
        }
    }
    let mut program = Vec::new();
    file.read_to_end(&mut program)?;
    Ok(sha256(&program))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    use sha2::Digest;
    sha2::Sha256::digest(bytes).into()
}

/// Registers the host with a browser for the current user (ADR-0006).
pub trait NativeHostRegistrar {
    /// Registers `host` (an absolute path) for the extension ids `allowed`
    /// (one at least), replacing an earlier registration.
    fn register(&self, browser: Browser, host: &Path, allowed: &[&str]) -> io::Result<()>;
    /// Removes the registration; nothing registered is not an error.
    fn unregister(&self, browser: Browser) -> io::Result<()>;
    /// Whether Zajil's host is registered with `browser`.
    fn is_registered(&self, browser: Browser) -> io::Result<bool>;
}

/// The registrar of this system: manifest files, and on Windows the
/// registry under `HKCU`.
pub struct SystemRegistrar {
    /// macOS, Linux: the user's home. Windows: unused.
    #[cfg_attr(windows, allow(dead_code))]
    home: PathBuf,
    /// Linux: where Chrome, Chromium and Edge keep their folders.
    #[cfg_attr(not(all(unix, not(target_os = "macos"))), allow(dead_code))]
    chrome_config: PathBuf,
    /// Where the records of what registering made go, and on Windows the
    /// manifests: `<data dir>/native-messaging`.
    manifests: PathBuf,
    /// Windows: the key under `HKCU` the vendors' keys are in
    /// (`Software`; another one for tests).
    #[cfg_attr(unix, allow(dead_code))]
    registry_root: String,
}

impl SystemRegistrar {
    /// For the current user: their home; on Linux the configuration folder
    /// Chromium reads; `<data_dir>/native-messaging` for the records (and
    /// on Windows the manifests); the keys under `HKCU\Software`.
    pub fn for_current_user(data_dir: &Path) -> io::Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
        let chrome_config = chrome_config_home(
            &home,
            std::env::var_os("CHROME_CONFIG_HOME"),
            std::env::var_os("XDG_CONFIG_HOME"),
        );
        Ok(SystemRegistrar {
            home,
            chrome_config,
            manifests: data_dir.join("native-messaging"),
            registry_root: "Software".into(),
        })
    }

    /// Under another home (its `.config` for Chromium's folders), manifests
    /// folder and registry root (tests).
    pub fn under(home: &Path, manifests: &Path, registry_root: &str) -> Self {
        SystemRegistrar {
            home: home.to_path_buf(),
            chrome_config: home.join(".config"),
            manifests: manifests.to_path_buf(),
            registry_root: registry_root.to_owned(),
        }
    }

    /// Where the manifest for `browser` is written.
    pub fn manifest_path(&self, browser: Browser) -> PathBuf {
        #[cfg(unix)]
        {
            let (base, folder, hosts) = self.place(browser);
            base.join(folder)
                .join(hosts)
                .join(format!("{NATIVE_HOST_NAME}.json"))
        }
        #[cfg(windows)]
        {
            self.manifests.join(format!("{}.json", browser.file_stem()))
        }
    }

    /// Windows: the key whose default value is the manifest's path, under
    /// `HKCU`. Elsewhere `None`.
    pub fn registry_key(&self, browser: Browser) -> Option<String> {
        #[cfg(windows)]
        {
            Some(format!(
                r"{}\{}\NativeMessagingHosts\{NATIVE_HOST_NAME}",
                self.registry_root,
                browser.vendor_key()
            ))
        }
        #[cfg(unix)]
        {
            let _ = browser;
            None
        }
    }

    /// macOS, Linux: (a folder never removed, the browser's folder in it,
    /// the manifests' folder in that).
    #[cfg(unix)]
    fn place(&self, browser: Browser) -> (PathBuf, &'static str, &'static str) {
        const HOSTS: &str = "NativeMessagingHosts";
        if cfg!(target_os = "macos") {
            let base = self.home.join("Library/Application Support");
            let folder = match browser {
                Browser::Chrome => "Google/Chrome",
                Browser::Chromium => "Chromium",
                Browser::Edge => "Microsoft Edge",
                Browser::Firefox => "Mozilla",
            };
            (base, folder, HOSTS)
        } else {
            match browser {
                Browser::Chrome => (self.chrome_config.clone(), "google-chrome", HOSTS),
                Browser::Chromium => (self.chrome_config.clone(), "chromium", HOSTS),
                Browser::Edge => (self.chrome_config.clone(), "microsoft-edge", HOSTS),
                Browser::Firefox => (self.home.clone(), ".mozilla", "native-messaging-hosts"),
            }
        }
    }

    /// The folders registering may create for `browser`, outermost first:
    /// (a folder they are relative to, each one as a path relative to it,
    /// `.` for that folder itself). On Windows none outside the data folder.
    fn folder_levels(&self, browser: Browser) -> (PathBuf, Vec<String>) {
        #[cfg(unix)]
        {
            let (base, folder, hosts) = self.place(browser);
            let mut levels = vec![".".to_owned()];
            let mut path = String::new();
            for part in folder.split('/').chain([hosts]) {
                if !path.is_empty() {
                    path.push('/');
                }
                path.push_str(part);
                levels.push(path.clone());
            }
            (base, levels)
        }
        #[cfg(windows)]
        {
            let _ = browser;
            (self.manifests.clone(), Vec::new())
        }
    }

    /// Windows: the keys registering may create above Zajil's own key,
    /// outermost first, relative to the registry root. Elsewhere none.
    #[cfg_attr(unix, allow(dead_code))]
    fn key_levels(&self, browser: Browser) -> Vec<String> {
        #[cfg(windows)]
        {
            let mut levels = Vec::new();
            let mut path = String::new();
            for part in browser
                .vendor_key()
                .split('\\')
                .chain(["NativeMessagingHosts"])
            {
                if !path.is_empty() {
                    path.push('\\');
                }
                path.push_str(part);
                levels.push(path.clone());
            }
            levels
        }
        #[cfg(unix)]
        {
            let _ = browser;
            Vec::new()
        }
    }

    fn record_path(&self, browser: Browser) -> PathBuf {
        self.manifests.join(format!("{}.made", browser.file_stem()))
    }
}

/// What registering made for one browser: lines `folder <level>` and
/// `key <level>`, levels as [`SystemRegistrar::folder_levels`] and
/// [`SystemRegistrar::key_levels`] give them.
#[derive(Default)]
struct Made {
    folders: Vec<String>,
    keys: Vec<String>,
}

impl Made {
    /// The largest record read; a real one is a few hundred bytes.
    const MAX: u64 = 4096;

    fn read(path: &Path) -> io::Result<Made> {
        use std::io::Read;
        let file = match crate::fs::open_regular_nofollow(path, false) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(Made::default()),
            Err(err) => return Err(err),
        };
        let mut text = String::new();
        file.take(Made::MAX).read_to_string(&mut text)?;
        let mut made = Made::default();
        for line in text.lines() {
            match line.split_once(' ') {
                Some(("folder", level)) => made.folders.push(level.to_owned()),
                Some(("key", level)) => made.keys.push(level.to_owned()),
                _ => {}
            }
        }
        Ok(made)
    }

    fn write(&self, path: &Path) -> io::Result<()> {
        let mut text = String::new();
        for (kind, levels) in [("folder", &self.folders), ("key", &self.keys)] {
            for level in levels {
                text.push_str(&format!("{kind} {level}\n"));
            }
        }
        write_new_then_replace(path, text.as_bytes())
    }

    fn add(list: &mut Vec<String>, level: &str) {
        if !list.iter().any(|l| l == level) {
            list.push(level.to_owned());
        }
    }
}

impl NativeHostRegistrar for SystemRegistrar {
    fn register(&self, browser: Browser, host: &Path, allowed: &[&str]) -> io::Result<()> {
        if !host.is_absolute() {
            return Err(invalid("the host's path must be absolute"));
        }
        // A path the manifest cannot hold exactly would name another file.
        let host = host
            .to_str()
            .ok_or_else(|| invalid("the host's path is not valid Unicode"))?;
        if allowed.is_empty() {
            return Err(invalid("no extension is allowed"));
        }
        for id in allowed {
            check_id(browser, id)?;
        }
        // What is missing is recorded before it is made, so that a
        // registration cut short leaves nothing unregistering would keep.
        let (base, folders) = self.folder_levels(browser);
        let mut made = Made::read(&self.record_path(browser))?;
        let missing: Vec<&String> = folders
            .iter()
            .filter(|level| at(&base, level).symlink_metadata().is_err())
            .collect();
        for level in &missing {
            Made::add(&mut made.folders, level);
        }
        #[cfg(windows)]
        for level in self.key_levels(browser) {
            if !imp::key_exists(&format!(r"{}\{level}", self.registry_root))? {
                Made::add(&mut made.keys, &level);
            }
        }
        let path = self.manifest_path(browser);
        let manifest = manifest_json(browser, host, allowed);
        // Registered already, exactly so: nothing is written (the app
        // registers at each start).
        let same_file = path.symlink_metadata().is_ok_and(|m| m.is_file())
            && std::fs::read(&path).is_ok_and(|bytes| bytes == manifest.as_bytes());
        #[cfg(windows)]
        let same_file = same_file
            && imp::get_default(&self.registry_key(browser).expect("Windows"))?.as_deref()
                == path.to_str();
        if missing.is_empty() && same_file {
            return Ok(());
        }
        std::fs::create_dir_all(&self.manifests)?;
        if !made.folders.is_empty() || !made.keys.is_empty() {
            made.write(&self.record_path(browser))?;
        }
        for level in missing {
            make_private_dir(&at(&base, level))?;
        }
        write_new_then_replace(&path, manifest.as_bytes())?;
        #[cfg(windows)]
        imp::set_default(
            &self.registry_key(browser).expect("Windows"),
            path.to_str().expect("made of valid Unicode"),
        )?;
        Ok(())
    }

    fn unregister(&self, browser: Browser) -> io::Result<()> {
        let record = self.record_path(browser);
        let made = Made::read(&record)?;
        let path = self.manifest_path(browser);
        remove_if_there(&path)?;
        // Left by a registration that stopped before its rename.
        remove_leftovers(&path)?;
        remove_leftovers(&record)?;
        #[cfg(windows)]
        {
            imp::delete_key(&self.registry_key(browser).expect("Windows"))?;
            for level in self.key_levels(browser).iter().rev() {
                if made.keys.contains(level) {
                    imp::delete_if_empty(&format!(r"{}\{level}", self.registry_root))?;
                }
            }
        }
        remove_if_there(&record)?;
        // Innermost first; `remove_dir` fails, harmlessly, on a folder
        // holding anything, and never follows a link.
        let (base, folders) = self.folder_levels(browser);
        for level in folders.iter().rev() {
            if made.folders.contains(level) {
                let _ = std::fs::remove_dir(at(&base, level));
            }
        }
        // Zajil's own folder in the data folder, once no record is left.
        let _ = std::fs::remove_dir(&self.manifests);
        Ok(())
    }

    fn is_registered(&self, browser: Browser) -> io::Result<bool> {
        #[cfg(windows)]
        if imp::get_default(&self.registry_key(browser).expect("Windows"))?.is_none() {
            return Ok(false);
        }
        Ok(self.manifest_path(browser).is_file())
    }
}

/// Linux: where Chromium-based browsers keep their folders, as Chromium
/// reads it (`docs/user_data_dir.md`); relative values are ignored, as
/// the XDG specification asks.
fn chrome_config_home(
    home: &Path,
    chrome_config_home: Option<std::ffi::OsString>,
    xdg_config_home: Option<std::ffi::OsString>,
) -> PathBuf {
    [chrome_config_home, xdg_config_home]
        .into_iter()
        .flatten()
        .map(PathBuf::from)
        .find(|path| path.is_absolute())
        .unwrap_or_else(|| home.join(".config"))
}

/// A folder level of [`SystemRegistrar::folder_levels`] as a path.
fn at(base: &Path, level: &str) -> PathBuf {
    if level == "." {
        base.to_path_buf()
    } else {
        base.join(level)
    }
}

/// `<name>.tmp` next to `path` (`chrome.json.tmp`, `chrome.made.tmp`).
/// `<name>.<process>-<n>.tmp` next to `path`: its own for each write, so
/// two registrations at once (two Zajil windows, or the start and the
/// settings button) never take each other's temporary file.
fn tmp_path(path: &Path) -> PathBuf {
    use std::sync::atomic::{AtomicU64, Ordering};
    static NEXT: AtomicU64 = AtomicU64::new(0);
    let mut name = path.file_name().expect("a file").to_os_string();
    name.push(format!(
        ".{}-{}.tmp",
        std::process::id(),
        NEXT.fetch_add(1, Ordering::Relaxed)
    ));
    path.with_file_name(name)
}

/// Removes the temporary files of `path` that writes cut short left
/// (`<name>.*.tmp`), links included and never followed.
fn remove_leftovers(path: &Path) -> io::Result<()> {
    let (Some(dir), Some(name)) = (path.parent(), path.file_name().and_then(|n| n.to_str())) else {
        return Ok(());
    };
    let entries = match std::fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(err) if err.kind() == io::ErrorKind::NotFound => return Ok(()),
        Err(err) => return Err(err),
    };
    let prefix = format!("{name}.");
    for entry in entries {
        let entry = entry?;
        let file = entry.file_name();
        let file = file.to_string_lossy();
        if file.starts_with(&prefix) && file.ends_with(".tmp") {
            remove_if_there(&entry.path())?;
        }
    }
    Ok(())
}

/// Writes `bytes` to a new private temporary file next to `path` (a link
/// in its place is refused), then puts it in place of `path`; the
/// temporary file is removed if anything fails.
fn write_new_then_replace(path: &Path, bytes: &[u8]) -> io::Result<()> {
    let tmp = tmp_path(path);
    remove_if_there(&tmp)?;
    let written = crate::fs::create_private(&tmp)
        .and_then(|mut file| {
            file.write_all(bytes)?;
            file.sync_all()
        })
        .and_then(|()| crate::fs::replace_file(&tmp, path));
    if written.is_err() {
        let _ = std::fs::remove_file(&tmp);
    }
    written
}

/// A folder only its owner may open on macOS and Linux (`0700`, as the
/// browsers make theirs), with its missing parents.
fn make_private_dir(path: &Path) -> io::Result<()> {
    let mut builder = std::fs::DirBuilder::new();
    builder.recursive(true);
    #[cfg(unix)]
    std::os::unix::fs::DirBuilderExt::mode(&mut builder, 0o700);
    builder.create(path)
}

/// Removes the file or link at `path` (never what a link points to),
/// retrying while another program holds it for a moment (Windows).
fn remove_if_there(path: &Path) -> io::Result<()> {
    crate::fs::with_retry(|| match std::fs::remove_file(path) {
        Err(err) if err.kind() != io::ErrorKind::NotFound => Err(err),
        _ => Ok(()),
    })
}

fn invalid(why: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidInput, why.to_owned())
}

/// Chrome-family ids are 32 letters `a`–`p`; Firefox add-on ids are
/// `name@domain` (letters, digits, `.`, `-`, `_`) or a `{uuid}`.
fn check_id(browser: Browser, id: &str) -> io::Result<()> {
    let ok = if browser.is_firefox() {
        let email = id.split_once('@').is_some_and(|(name, domain)| {
            let part = |s: &str| {
                !s.is_empty()
                    && s.bytes()
                        .all(|b| b.is_ascii_alphanumeric() || b"._-".contains(&b))
            };
            part(name) && part(domain) && domain.contains('.')
        });
        let uuid = id.len() == 38
            && id.starts_with('{')
            && id.ends_with('}')
            && id[1..37].bytes().enumerate().all(|(i, b)| {
                if [8, 13, 18, 23].contains(&i) {
                    b == b'-'
                } else {
                    b.is_ascii_hexdigit()
                }
            });
        email || uuid
    } else {
        id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
    };
    if ok {
        Ok(())
    } else {
        Err(invalid(&format!(
            "{id:?} is not an extension id for {browser:?}"
        )))
    }
}

/// The manifest, as each browser documents it.
fn manifest_json(browser: Browser, host: &str, allowed: &[&str]) -> String {
    let list: Vec<String> = allowed
        .iter()
        .map(|id| {
            if browser.is_firefox() {
                json_string(id)
            } else {
                json_string(&format!("chrome-extension://{id}/"))
            }
        })
        .collect();
    let key = if browser.is_firefox() {
        "allowed_extensions"
    } else {
        "allowed_origins"
    };
    format!(
        "{{\n  \"name\": {},\n  \"description\": \"Zajil download manager\",\n  \"path\": {},\n  \"type\": \"stdio\",\n  \"{key}\": [{}]\n}}\n",
        json_string(NATIVE_HOST_NAME),
        json_string(host),
        list.join(", ")
    )
}

/// A JSON string (paths on Windows have backslashes).
fn json_string(text: &str) -> String {
    let mut out = String::with_capacity(text.len() + 2);
    out.push('"');
    for c in text.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

#[cfg(windows)]
#[allow(unsafe_code)]
mod imp {
    use std::io;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_MORE_DATA, ERROR_SUCCESS};
    use windows_sys::Win32::System::Registry::{
        HKEY, HKEY_CURRENT_USER, KEY_READ, KEY_SET_VALUE, REG_OPTION_NON_VOLATILE, REG_SZ,
        RRF_RT_REG_SZ, RegCloseKey, RegCreateKeyExW, RegDeleteKeyW, RegGetValueW, RegOpenKeyExW,
        RegQueryInfoKeyW, RegSetValueExW,
    };

    fn wide(text: &str) -> Vec<u16> {
        text.encode_utf16().chain(Some(0)).collect()
    }

    fn check(status: u32) -> io::Result<()> {
        if status == ERROR_SUCCESS {
            Ok(())
        } else {
            Err(io::Error::from_raw_os_error(status as i32))
        }
    }

    struct Key(HKEY);

    impl Drop for Key {
        fn drop(&mut self) {
            // SAFETY: the key was opened by us and is closed once.
            unsafe { RegCloseKey(self.0) };
        }
    }

    fn open(key: &str, access: u32) -> io::Result<Option<Key>> {
        let name = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and alive for the call; `handle`
        // is a writable out-pointer.
        let status =
            unsafe { RegOpenKeyExW(HKEY_CURRENT_USER, name.as_ptr(), 0, access, &mut handle) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status)?;
        Ok(Some(Key(handle)))
    }

    pub fn key_exists(key: &str) -> io::Result<bool> {
        Ok(open(key, KEY_READ)?.is_some())
    }

    pub fn set_default(key: &str, value: &str) -> io::Result<()> {
        let name = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: `name` is NUL-terminated and alive for the call; the class
        // and security pointers are optional; `handle` is an out-pointer.
        check(unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        })?;
        let key = Key(handle);
        let data = wide(value);
        // SAFETY: `data` is a NUL-terminated UTF-16 string; its size in bytes
        // includes the terminator, as REG_SZ requires. A null value name is
        // the key's default value.
        check(unsafe {
            RegSetValueExW(
                key.0,
                std::ptr::null(),
                0,
                REG_SZ,
                data.as_ptr().cast(),
                (data.len() * 2) as u32,
            )
        })
    }

    pub fn get_default(key: &str) -> io::Result<Option<String>> {
        let name = wide(key);
        let read = |buffer: Option<&mut [u16]>, size: &mut u32| {
            let data = buffer.map_or(std::ptr::null_mut(), |b| b.as_mut_ptr().cast());
            // SAFETY: `name` is NUL-terminated; `data` is null (asking for
            // the size) or holds `size` bytes; the type pointer is optional.
            unsafe {
                RegGetValueW(
                    HKEY_CURRENT_USER,
                    name.as_ptr(),
                    std::ptr::null(),
                    RRF_RT_REG_SZ,
                    std::ptr::null_mut(),
                    data,
                    size,
                )
            }
        };
        // The size first, then the value; again if it grew in between.
        loop {
            let mut size = 0u32;
            let status = read(None, &mut size);
            if status == ERROR_FILE_NOT_FOUND {
                return Ok(None);
            }
            check(status)?;
            let mut buffer = vec![0u16; (size as usize).div_ceil(2)];
            let status = read(Some(&mut buffer), &mut size);
            match status {
                ERROR_MORE_DATA => continue,
                ERROR_FILE_NOT_FOUND => return Ok(None),
                _ => check(status)?,
            }
            // `size` counts bytes and the terminating NUL.
            let chars = (size as usize / 2).saturating_sub(1);
            return Ok(Some(String::from_utf16_lossy(&buffer[..chars])));
        }
    }

    pub fn delete_key(key: &str) -> io::Result<()> {
        let name = wide(key);
        // SAFETY: `name` is NUL-terminated and alive for the call.
        let status = unsafe { RegDeleteKeyW(HKEY_CURRENT_USER, name.as_ptr()) };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(());
        }
        check(status)
    }

    /// Deletes `key` if it has no subkey and no value; whether it is gone.
    pub fn delete_if_empty(key: &str) -> io::Result<bool> {
        let Some(open) = open(key, KEY_READ)? else {
            return Ok(true);
        };
        let (mut subkeys, mut values) = (0u32, 0u32);
        // SAFETY: the key is open; every other out-pointer is optional.
        check(unsafe {
            RegQueryInfoKeyW(
                open.0,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null(),
                &mut subkeys,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                &mut values,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                std::ptr::null_mut(),
            )
        })?;
        drop(open);
        if subkeys == 0 && values == 0 {
            delete_key(key)?;
            return Ok(true);
        }
        Ok(false)
    }

    /// Tests: removes a whole test root.
    #[cfg(test)]
    pub fn delete_tree(key: &str) {
        use windows_sys::Win32::System::Registry::RegDeleteTreeW;
        let name = wide(key);
        // SAFETY: `name` is NUL-terminated and alive for the call.
        unsafe { RegDeleteTreeW(HKEY_CURRENT_USER, name.as_ptr()) };
        let _ = delete_key(key);
    }

    /// Tests: creates a key (a browser "installed").
    #[cfg(test)]
    pub fn create(key: &str) {
        let name = wide(key);
        let mut handle: HKEY = std::ptr::null_mut();
        // SAFETY: as in `set_default`.
        let status = unsafe {
            RegCreateKeyExW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                0,
                std::ptr::null(),
                REG_OPTION_NON_VOLATILE,
                KEY_SET_VALUE,
                std::ptr::null(),
                &mut handle,
                std::ptr::null_mut(),
            )
        };
        check(status).unwrap();
        drop(Key(handle));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const CHROME_ID: &str = "abcdefghijklmnopabcdefghijklmnop";
    const FIREFOX_ID: &str = "zajil@motalim-ai.github.io";

    fn tempdir() -> PathBuf {
        use std::sync::atomic::{AtomicU32, Ordering};
        static N: AtomicU32 = AtomicU32::new(0);
        let dir = std::env::temp_dir().join(format!(
            "zajil-platform-native-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn id_for(browser: Browser) -> &'static str {
        if browser.is_firefox() {
            FIREFOX_ID
        } else {
            CHROME_ID
        }
    }

    /// A test home with a data folder (and on Windows a test registry
    /// root), all removed on drop, also when a test fails.
    struct World {
        registrar: SystemRegistrar,
        #[cfg_attr(unix, allow(dead_code))]
        root: String,
        base: PathBuf,
        /// What was there before the test: the folders every system has.
        initial: Vec<PathBuf>,
    }

    impl World {
        fn new() -> Self {
            let base = tempdir();
            // Unique as the folder is: Software\ZajilTest-<pid>-<n>.
            let root = format!(
                r"Software\ZajilTest-{}",
                base.file_name().unwrap().to_string_lossy()
            );
            // The folders a real home always has, never removed.
            let always = if cfg!(target_os = "macos") {
                "home/Library/Application Support"
            } else {
                "home/.config"
            };
            std::fs::create_dir_all(base.join(always)).unwrap();
            std::fs::create_dir_all(base.join("data")).unwrap();
            let registrar = SystemRegistrar::under(
                &base.join("home"),
                &base.join("data/native-messaging"),
                &root,
            );
            let mut world = World {
                registrar,
                root,
                base,
                initial: Vec::new(),
            };
            world.initial = world.entries();
            world
        }

        /// What the test added to the home and data folder.
        fn added(&self) -> Vec<PathBuf> {
            let mut added = self.entries();
            added.retain(|path| !self.initial.contains(path));
            added
        }

        /// Every file and folder under the test home and data folder.
        fn entries(&self) -> Vec<PathBuf> {
            fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        out.push(path.clone());
                        if path.is_dir() {
                            walk(&path, out);
                        }
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.base.join("home"), &mut out);
            walk(&self.base.join("data"), &mut out);
            out
        }
    }

    impl Drop for World {
        fn drop(&mut self) {
            #[cfg(windows)]
            imp::delete_tree(&self.root);
            let _ = std::fs::remove_dir_all(&self.base);
        }
    }

    fn host() -> PathBuf {
        std::env::temp_dir().join(if cfg!(windows) {
            "Zajil \"x\"\\zajil-host.exe"
        } else {
            "Zajil/zajil-host"
        })
    }

    /// The M5 criterion: each browser registered then unregistered, its
    /// manifest and (Windows) registry key right, and nothing left. In an
    /// empty home: a browser not installed, or never run yet (on macOS
    /// Firefox's own folder is `Firefox`, not `Mozilla`), is registered too.
    #[test]
    fn each_browser_is_registered_then_unregistered_leaving_nothing() {
        let world = World::new();
        let r = &world.registrar;
        for browser in Browser::ALL {
            r.register(browser, &host(), &[id_for(browser)]).unwrap();
            assert!(r.is_registered(browser).unwrap(), "{browser:?}");
            let manifest = std::fs::read_to_string(r.manifest_path(browser)).unwrap();
            assert!(
                manifest.contains(&format!("\"name\": \"{NATIVE_HOST_NAME}\"")),
                "{manifest}"
            );
            assert!(
                manifest.contains(&format!(
                    "\"path\": {}",
                    json_string(host().to_str().unwrap())
                )),
                "{manifest}"
            );
            assert!(manifest.contains("\"type\": \"stdio\""));
            if browser.is_firefox() {
                assert!(
                    manifest.contains(&format!("\"allowed_extensions\": [\"{FIREFOX_ID}\"]")),
                    "{manifest}"
                );
            } else {
                assert!(
                    manifest.contains(&format!(
                        "\"allowed_origins\": [\"chrome-extension://{CHROME_ID}/\"]"
                    )),
                    "{manifest}"
                );
            }
            #[cfg(windows)]
            assert_eq!(
                imp::get_default(&r.registry_key(browser).unwrap())
                    .unwrap()
                    .map(PathBuf::from),
                Some(r.manifest_path(browser))
            );
        }
        for browser in Browser::ALL {
            r.unregister(browser).unwrap();
            assert!(!r.is_registered(browser).unwrap(), "{browser:?}");
            #[cfg(windows)]
            assert_eq!(
                imp::get_default(&r.registry_key(browser).unwrap()).unwrap(),
                None
            );
            // Again: nothing registered is not an error.
            r.unregister(browser).unwrap();
        }
        assert_eq!(world.added(), Vec::<PathBuf>::new(), "left behind");
        #[cfg(windows)]
        assert!(
            imp::delete_if_empty(&world.root).unwrap(),
            "keys left under the test root"
        );
    }

    /// What was there before registering stays after unregistering: the
    /// browser's own folder or key and another host's manifest.
    #[test]
    fn what_was_there_before_is_kept() {
        let world = World::new();
        let r = &world.registrar;
        let manifest = r.manifest_path(Browser::Chrome);
        let hosts = manifest.parent().unwrap().to_path_buf();
        let browser_folder = hosts.parent().unwrap().to_path_buf();
        // Empty, as a folder or key made by someone else before Zajil.
        std::fs::create_dir_all(&browser_folder).unwrap();
        #[cfg(windows)]
        imp::create(&format!(r"{}\{}", world.root, Browser::Chrome.vendor_key()));

        r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
        r.unregister(Browser::Chrome).unwrap();
        assert!(browser_folder.is_dir());
        #[cfg(windows)]
        assert!(
            imp::key_exists(&format!(r"{}\{}", world.root, Browser::Chrome.vendor_key())).unwrap()
        );

        // What the browser puts in a folder Zajil made stays too.
        let world = World::new();
        let r = &world.registrar;
        r.register(Browser::Edge, &host(), &[CHROME_ID]).unwrap();
        let profile = r
            .manifest_path(Browser::Edge)
            .parent()
            .unwrap()
            .parent()
            .unwrap()
            .join("Default");
        std::fs::create_dir_all(&profile).unwrap();
        r.unregister(Browser::Edge).unwrap();
        assert!(profile.is_dir());
        let world = World::new();
        let r = &world.registrar;
        let manifest = r.manifest_path(Browser::Chrome);
        r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
        let other = manifest.with_file_name("com.example.other.json");
        std::fs::write(&other, "{}").unwrap();
        r.unregister(Browser::Chrome).unwrap();
        assert!(other.is_file());
        assert!(!manifest.exists());
    }

    /// A registration that stopped before its rename leaves its temporary
    /// file; registering again and unregistering remove it.
    #[test]
    fn a_leftover_temporary_file_is_removed() {
        let world = World::new();
        let r = &world.registrar;
        r.register(Browser::Edge, &host(), &[CHROME_ID]).unwrap();
        let tmp = leftover(&r.manifest_path(Browser::Edge));
        let record_tmp = leftover(&r.record_path(Browser::Edge));
        std::fs::write(&tmp, "half").unwrap();
        std::fs::write(&record_tmp, "half").unwrap();
        // Another Zajil may be writing it: registering leaves it.
        r.register(Browser::Edge, &host(), &[CHROME_ID]).unwrap();
        assert!(tmp.exists());
        r.unregister(Browser::Edge).unwrap();
        assert_eq!(world.added(), Vec::<PathBuf>::new(), "left behind");
    }

    /// A temporary file another registration (or one cut short) left.
    fn leftover(path: &Path) -> PathBuf {
        let mut name = path.file_name().unwrap().to_os_string();
        name.push(".999999-0.tmp");
        path.with_file_name(name)
    }

    /// Four registrations at once (two Zajil windows, the start and the
    /// settings button) all succeed, and leave one valid manifest.
    #[test]
    fn registrations_at_once_do_not_take_each_other_s_files() {
        let world = World::new();
        std::thread::scope(|scope| {
            for _ in 0..4 {
                scope.spawn(|| {
                    for round in 0..50 {
                        // A different host each round, so each one writes.
                        let host = host().with_file_name(format!("zajil-host-{round}"));
                        world
                            .registrar
                            .register(Browser::Chrome, &host, &[CHROME_ID])
                            .unwrap();
                    }
                });
            }
        });
        let manifest =
            std::fs::read_to_string(world.registrar.manifest_path(Browser::Chrome)).unwrap();
        assert!(manifest.contains("zajil-host-49"), "{manifest}");
        for browser in Browser::ALL {
            world.registrar.unregister(browser).unwrap();
        }
        assert_eq!(world.added(), Vec::<PathBuf>::new(), "left behind");
    }

    /// Registering again, unchanged, writes nothing; a change is written.
    #[test]
    fn an_unchanged_registration_is_not_written_again() {
        let world = World::new();
        let r = &world.registrar;
        let manifest = r.manifest_path(Browser::Firefox);
        r.register(Browser::Firefox, &host(), &[FIREFOX_ID])
            .unwrap();
        let before = std::fs::symlink_metadata(&manifest).unwrap();
        r.register(Browser::Firefox, &host(), &[FIREFOX_ID])
            .unwrap();
        let after = std::fs::symlink_metadata(&manifest).unwrap();
        #[cfg(unix)]
        {
            use std::os::unix::fs::MetadataExt;
            assert_eq!(before.ino(), after.ino(), "replaced");
        }
        assert_eq!(before.modified().unwrap(), after.modified().unwrap());
        let other = host().with_file_name("other-host");
        r.register(Browser::Firefox, &other, &[FIREFOX_ID]).unwrap();
        assert!(
            std::fs::read_to_string(&manifest)
                .unwrap()
                .contains("other-host")
        );
    }

    /// A symbolic or hard link in place of the manifest or its temporary
    /// file never changes the file it points to (AGENTS.md §10, item 1):
    /// the rename replaces the link itself, the temporary file is created
    /// new, and removing removes the link.
    #[test]
    fn links_in_place_of_the_manifest_leave_their_target_alone() {
        let world = World::new();
        let r = &world.registrar;
        let manifest = r.manifest_path(Browser::Chrome);
        let outside = world.base.join("outside.json");
        let link = |at: &Path, hard: bool| {
            // Unregistering removed the empty folder.
            std::fs::create_dir_all(at.parent().unwrap()).unwrap();
            std::fs::write(&outside, "keep").unwrap();
            if hard {
                std::fs::hard_link(&outside, at).unwrap();
            } else {
                crate::fs::symlink_file(&outside, at).expect("a symbolic link for the test");
            }
        };
        for hard in [false, true] {
            for at in [manifest.clone(), leftover(&manifest)] {
                link(&at, hard);
                r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
                assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep", "{at:?}");
                r.unregister(Browser::Chrome).unwrap();
                let _ = std::fs::remove_file(&at);
            }
            link(&manifest, hard);
            r.unregister(Browser::Chrome).unwrap();
            assert!(!manifest.exists() && manifest.symlink_metadata().is_err());
            assert_eq!(std::fs::read_to_string(&outside).unwrap(), "keep");
        }
    }

    /// A base folder missing before registering (an empty home's
    /// `~/.config`) is made, then removed again; the folders registering
    /// makes are the user's only, as the browsers make theirs.
    #[cfg(unix)]
    #[test]
    fn folders_made_are_private_and_removed_again() {
        use std::os::unix::fs::PermissionsExt;
        let world = World::new();
        let r = &world.registrar;
        let (base, levels) = r.folder_levels(Browser::Chrome);
        std::fs::remove_dir(&base).unwrap();
        r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
        for level in &levels {
            let mode = at(&base, level).metadata().unwrap().permissions().mode();
            assert_eq!(mode & 0o777, 0o700, "{level}");
        }
        r.unregister(Browser::Chrome).unwrap();
        assert!(!base.exists());
        assert_eq!(world.added(), Vec::<PathBuf>::new(), "left behind");
    }

    /// A record naming anything but the browser's own levels is ignored:
    /// an empty folder elsewhere is never removed.
    #[test]
    fn a_record_naming_other_folders_is_ignored() {
        let world = World::new();
        let r = &world.registrar;
        let outside = world.base.join("home/outside");
        std::fs::create_dir_all(&outside).unwrap();
        r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
        let record = r.record_path(Browser::Chrome);
        let mut text = std::fs::read_to_string(&record).unwrap();
        let (base, _) = r.folder_levels(Browser::Chrome);
        for level in ["../outside", "../../outside", "../home/outside"] {
            text.push_str(&format!("folder {level}\n"));
        }
        text.push_str(&format!("folder {}\nkey ..\\Other\n", outside.display()));
        std::fs::write(&record, text).unwrap();
        r.unregister(Browser::Chrome).unwrap();
        assert!(outside.is_dir(), "relative to {base:?}");
    }

    #[test]
    fn extension_ids_and_the_host_path_are_checked() {
        let world = World::new();
        let r = &world.registrar;
        for (browser, id) in [
            (Browser::Chrome, "short"),
            (Browser::Chrome, "abcdefghijklmnopabcdefghijklmnoz"),
            (Browser::Edge, "abcdefghijklmnopabcdefghijklmnop\"]"),
            (Browser::Firefox, "no-at-sign"),
            (Browser::Firefox, "a@b\"c.d"),
            (Browser::Firefox, "{12345678-1234-1234-1234-12345678901}"),
        ] {
            let err = r.register(browser, &host(), &[id]).expect_err(id);
            assert_eq!(err.kind(), io::ErrorKind::InvalidInput, "{id}");
        }
        assert!(
            r.register(
                Browser::Firefox,
                &host(),
                &["{12345678-1234-1234-1234-123456789012}"]
            )
            .is_ok()
        );
        assert!(
            r.register(Browser::Chrome, Path::new("zajil-host"), &[CHROME_ID])
                .is_err()
        );
        assert!(r.register(Browser::Chrome, &host(), &[]).is_err());
    }

    /// A host path the manifest cannot hold exactly is refused, rather
    /// than written as the name of another file.
    #[cfg(unix)]
    #[test]
    fn a_host_path_that_is_not_unicode_is_refused() {
        use std::os::unix::ffi::OsStrExt;
        let world = World::new();
        let host = Path::new(std::ffi::OsStr::from_bytes(b"/opt/Zaj\xffil/zajil-host"));
        let err = world
            .registrar
            .register(Browser::Chrome, host, &[CHROME_ID])
            .unwrap_err();
        assert_eq!(err.kind(), io::ErrorKind::InvalidInput);
        assert_eq!(world.added(), Vec::<PathBuf>::new());
    }

    /// A registry value longer than any fixed buffer is read whole.
    #[cfg(windows)]
    #[test]
    fn a_long_registry_value_is_read_whole() {
        let world = World::new();
        let key = format!(r"{}\Long", world.root);
        let value = format!(r"C:\{}\zajil.json", "x".repeat(5000));
        imp::set_default(&key, &value).unwrap();
        assert_eq!(imp::get_default(&key).unwrap(), Some(value));
    }

    /// Linux: Chromium's folders follow `$CHROME_CONFIG_HOME`, then
    /// `$XDG_CONFIG_HOME`; relative values are ignored.
    #[test]
    fn chromium_s_configuration_folder_follows_the_environment() {
        // Absolute on every system.
        let root = std::env::temp_dir();
        let home = &root.join("home");
        let (cfg, c) = (root.join("cfg"), root.join("c"));
        let os = |p: &Path| Some(p.as_os_str().to_owned());
        assert_eq!(chrome_config_home(home, None, None), home.join(".config"));
        assert_eq!(chrome_config_home(home, None, os(&cfg)), cfg);
        assert_eq!(chrome_config_home(home, os(&c), os(&cfg)), c);
        let os = |s: &str| Some(std::ffi::OsString::from(s));
        assert_eq!(
            chrome_config_home(home, os("rel"), os("also-rel")),
            home.join(".config")
        );
    }

    /// The real places, copied here from each browser's documentation
    /// (Chrome, Microsoft Learn for Edge, MDN for Firefox).
    #[test]
    fn the_places_are_the_documented_ones() {
        let data = tempdir();
        let r = SystemRegistrar::under(
            Path::new("/home/u"),
            &data.join("native-messaging"),
            "Software",
        );
        let path = |b| r.manifest_path(b).to_string_lossy().replace('\\', "/");
        let name = NATIVE_HOST_NAME;
        if cfg!(target_os = "macos") {
            let support = "/home/u/Library/Application Support";
            for (browser, folder) in [
                (Browser::Chrome, "Google/Chrome/NativeMessagingHosts"),
                (Browser::Chromium, "Chromium/NativeMessagingHosts"),
                (Browser::Edge, "Microsoft Edge/NativeMessagingHosts"),
                (Browser::Firefox, "Mozilla/NativeMessagingHosts"),
            ] {
                assert_eq!(path(browser), format!("{support}/{folder}/{name}.json"));
            }
        } else if cfg!(windows) {
            for (browser, key, file) in [
                (Browser::Chrome, r"Google\Chrome", "chrome"),
                (Browser::Chromium, "Chromium", "chromium"),
                (Browser::Edge, r"Microsoft\Edge", "edge"),
                (Browser::Firefox, "Mozilla", "firefox"),
            ] {
                assert_eq!(
                    r.registry_key(browser).unwrap(),
                    format!(r"Software\{key}\NativeMessagingHosts\{name}")
                );
                assert_eq!(
                    r.manifest_path(browser),
                    data.join(format!(r"native-messaging\{file}.json"))
                );
            }
        } else {
            for (browser, folder) in [
                (
                    Browser::Chrome,
                    ".config/google-chrome/NativeMessagingHosts",
                ),
                (Browser::Chromium, ".config/chromium/NativeMessagingHosts"),
                (Browser::Edge, ".config/microsoft-edge/NativeMessagingHosts"),
                (Browser::Firefox, ".mozilla/native-messaging-hosts"),
            ] {
                assert_eq!(path(browser), format!("/home/u/{folder}/{name}.json"));
            }
        }
        let _ = std::fs::remove_dir_all(data);
    }
}
