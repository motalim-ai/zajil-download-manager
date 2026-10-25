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
//! | Linux | `~/.config/<browser>/NativeMessagingHosts/<name>.json`, Firefox `~/.mozilla/native-messaging-hosts/<name>.json` | its folder |
//! | Windows | `<data dir>/native-messaging/<browser>.json` | the default value of `HKCU\Software\<vendor>\NativeMessagingHosts\<name>` |
//!
//! A browser is registered only if it is there: its folder on macOS and
//! Linux, its vendor key in the registry on Windows. Nothing of a browser
//! that is not installed is ever created, so unregistering leaves nothing
//! behind: the manifest and the registry key go, and the
//! `NativeMessagingHosts` folder or key too when it is left empty.
//!
//! Extension ids are checked before they go into a manifest the browser
//! trusts: 32 letters `a`–`p` for Chrome-family ids; `name@domain` or a
//! `{uuid}` for Firefox.

use std::io;
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

    /// The browser's own folder under the user's home (macOS, Linux): its
    /// presence says it is installed; `NativeMessagingHosts` goes in it.
    #[cfg(unix)]
    fn folder(self) -> (&'static str, &'static str) {
        // (the browser's folder, the manifests' folder inside it)
        if cfg!(target_os = "macos") {
            match self {
                Browser::Chrome => (
                    "Library/Application Support/Google/Chrome",
                    "NativeMessagingHosts",
                ),
                Browser::Chromium => (
                    "Library/Application Support/Chromium",
                    "NativeMessagingHosts",
                ),
                Browser::Edge => (
                    "Library/Application Support/Microsoft Edge",
                    "NativeMessagingHosts",
                ),
                Browser::Firefox => (
                    "Library/Application Support/Mozilla",
                    "NativeMessagingHosts",
                ),
            }
        } else {
            match self {
                Browser::Chrome => (".config/google-chrome", "NativeMessagingHosts"),
                Browser::Chromium => (".config/chromium", "NativeMessagingHosts"),
                Browser::Edge => (".config/microsoft-edge", "NativeMessagingHosts"),
                Browser::Firefox => (".mozilla", "native-messaging-hosts"),
            }
        }
    }

    /// The vendor key under `HKCU\<root>` (Windows): its presence says the
    /// browser is installed; `NativeMessagingHosts` goes under it.
    #[cfg(windows)]
    fn vendor_key(self) -> &'static str {
        match self {
            Browser::Chrome => r"Google\Chrome",
            Browser::Chromium => "Chromium",
            Browser::Edge => r"Microsoft\Edge",
            Browser::Firefox => "Mozilla",
        }
    }

    #[cfg(windows)]
    fn file_stem(self) -> &'static str {
        match self {
            Browser::Chrome => "chrome",
            Browser::Chromium => "chromium",
            Browser::Edge => "edge",
            Browser::Firefox => "firefox",
        }
    }
}

/// What [`Registrar::register`] did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Outcome {
    Registered,
    /// The browser is not installed for this user: nothing was written.
    NotInstalled,
}

/// Registers and unregisters the host for the current user.
pub struct Registrar {
    /// macOS, Linux: the user's home. Windows: unused.
    #[cfg_attr(windows, allow(dead_code))]
    home: PathBuf,
    /// Windows: where the manifests are written.
    #[cfg_attr(unix, allow(dead_code))]
    manifests: PathBuf,
    /// Windows: the key under `HKCU` the vendors' keys are in
    /// (`Software`; another one for tests).
    #[cfg_attr(unix, allow(dead_code))]
    registry_root: String,
}

impl Registrar {
    /// For the current user: their home; on Windows the manifests go in
    /// `<data_dir>/native-messaging`, and the keys under `HKCU\Software`.
    pub fn for_current_user(data_dir: &Path) -> io::Result<Self> {
        let home = dirs::home_dir()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "no home folder"))?;
        Ok(Registrar {
            home,
            manifests: data_dir.join("native-messaging"),
            registry_root: "Software".into(),
        })
    }

    /// Under another home, manifests folder and registry root (tests).
    pub fn under(home: &Path, manifests: &Path, registry_root: &str) -> Self {
        Registrar {
            home: home.to_path_buf(),
            manifests: manifests.to_path_buf(),
            registry_root: registry_root.to_owned(),
        }
    }

    /// Where the manifest for `browser` is written.
    pub fn manifest_path(&self, browser: Browser) -> PathBuf {
        #[cfg(unix)]
        {
            let (folder, hosts) = browser.folder();
            self.home
                .join(folder)
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

    /// Registers `host` (an absolute path) for the extension ids `allowed`
    /// (one at least), if `browser` is installed.
    pub fn register(&self, browser: Browser, host: &Path, allowed: &[&str]) -> io::Result<Outcome> {
        if !host.is_absolute() {
            return Err(invalid("the host's path must be absolute"));
        }
        if allowed.is_empty() {
            return Err(invalid("no extension is allowed"));
        }
        for id in allowed {
            check_id(browser, id)?;
        }
        if !self.installed(browser)? {
            return Ok(Outcome::NotInstalled);
        }
        let path = self.manifest_path(browser);
        let folder = path.parent().expect("a manifest is in a folder");
        std::fs::create_dir_all(folder)?;
        let tmp = path.with_extension("json.tmp");
        let _ = std::fs::remove_file(&tmp);
        std::fs::write(&tmp, manifest_json(browser, host, allowed))?;
        crate::fs::replace_file(&tmp, &path)?;
        #[cfg(windows)]
        imp::set_default(&self.registry_key(browser).expect("Windows"), &path)?;
        Ok(Outcome::Registered)
    }

    /// Removes the registration of `browser`: its manifest, its registry
    /// key, and the `NativeMessagingHosts` folder or key when left empty.
    /// Nothing registered is not an error.
    pub fn unregister(&self, browser: Browser) -> io::Result<()> {
        let path = self.manifest_path(browser);
        match std::fs::remove_file(&path) {
            Ok(()) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound => {}
            Err(err) => return Err(err),
        }
        #[cfg(unix)]
        if let Some(folder) = path.parent() {
            // Fails, harmlessly, while other hosts are registered there.
            let _ = std::fs::remove_dir(folder);
        }
        #[cfg(windows)]
        {
            let key = self.registry_key(browser).expect("Windows");
            imp::delete_key(&key)?;
            // The NativeMessagingHosts key, if no other host is under it.
            if let Some((parent, _)) = key.rsplit_once('\\') {
                imp::delete_if_empty(parent)?;
            }
            if let Some(folder) = path.parent() {
                let _ = std::fs::remove_dir(folder);
            }
        }
        Ok(())
    }

    /// Whether `browser` has Zajil's host registered.
    pub fn is_registered(&self, browser: Browser) -> io::Result<bool> {
        #[cfg(windows)]
        if imp::get_default(&self.registry_key(browser).expect("Windows"))?.is_none() {
            return Ok(false);
        }
        Ok(self.manifest_path(browser).is_file())
    }

    fn installed(&self, browser: Browser) -> io::Result<bool> {
        #[cfg(unix)]
        {
            Ok(self.home.join(browser.folder().0).is_dir())
        }
        #[cfg(windows)]
        {
            imp::key_exists(&format!(r"{}\{}", self.registry_root, browser.vendor_key()))
        }
    }
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
fn manifest_json(browser: Browser, host: &Path, allowed: &[&str]) -> String {
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
        json_string(&host.to_string_lossy()),
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
    use std::path::Path;

    use windows_sys::Win32::Foundation::{ERROR_FILE_NOT_FOUND, ERROR_SUCCESS};
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

    pub fn set_default(key: &str, value: &Path) -> io::Result<()> {
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
        let data = wide(&value.to_string_lossy());
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
        let mut buffer = vec![0u16; 4096];
        let mut size = (buffer.len() * 2) as u32;
        // SAFETY: `name` is NUL-terminated; `buffer` holds `size` bytes; the
        // type pointer is optional.
        let status = unsafe {
            RegGetValueW(
                HKEY_CURRENT_USER,
                name.as_ptr(),
                std::ptr::null(),
                RRF_RT_REG_SZ,
                std::ptr::null_mut(),
                buffer.as_mut_ptr().cast(),
                &mut size,
            )
        };
        if status == ERROR_FILE_NOT_FOUND {
            return Ok(None);
        }
        check(status)?;
        let chars = (size as usize / 2).saturating_sub(1);
        Ok(Some(String::from_utf16_lossy(&buffer[..chars])))
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

    pub fn delete_if_empty(key: &str) -> io::Result<()> {
        let Some(open) = open(key, KEY_READ)? else {
            return Ok(());
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
        }
        Ok(())
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

    /// A test home (and on Windows a test registry root), with the given
    /// browsers "installed".
    struct World {
        registrar: Registrar,
        #[cfg_attr(unix, allow(dead_code))]
        root: String,
        base: PathBuf,
    }

    impl World {
        fn new(installed: &[Browser]) -> Self {
            let base = tempdir();
            // Unique as the folder is: Software\ZajilTest-<pid>-<n>.
            let root = format!(
                r"Software\ZajilTest-{}",
                base.file_name().unwrap().to_string_lossy()
            );
            let registrar = Registrar::under(
                &base.join("home"),
                &base.join("data/native-messaging"),
                &root,
            );
            for browser in installed {
                #[cfg(unix)]
                std::fs::create_dir_all(base.join("home").join(browser.folder().0)).unwrap();
                #[cfg(windows)]
                imp::create(&format!(r"{root}\{}", browser.vendor_key()));
            }
            World {
                registrar,
                root,
                base,
            }
        }

        /// Every file under the test home and data folder.
        fn files(&self) -> Vec<PathBuf> {
            fn walk(dir: &Path, out: &mut Vec<PathBuf>) {
                if let Ok(entries) = std::fs::read_dir(dir) {
                    for entry in entries.flatten() {
                        let path = entry.path();
                        if path.is_dir() {
                            walk(&path, out);
                        } else {
                            out.push(path);
                        }
                    }
                }
            }
            let mut out = Vec::new();
            walk(&self.base, &mut out);
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
    /// manifest and (Windows) registry key right, and nothing left.
    #[test]
    fn each_browser_is_registered_then_unregistered_leaving_nothing() {
        let world = World::new(&Browser::ALL);
        let r = &world.registrar;
        for browser in Browser::ALL {
            let id = if browser.is_firefox() {
                FIREFOX_ID
            } else {
                CHROME_ID
            };
            assert_eq!(
                r.register(browser, &host(), &[id]).unwrap(),
                Outcome::Registered,
                "{browser:?}"
            );
            assert!(r.is_registered(browser).unwrap(), "{browser:?}");
            let manifest = std::fs::read_to_string(r.manifest_path(browser)).unwrap();
            assert!(
                manifest.contains(&format!("\"name\": \"{NATIVE_HOST_NAME}\"")),
                "{manifest}"
            );
            assert!(
                manifest.contains(&format!(
                    "\"path\": {}",
                    json_string(&host().to_string_lossy())
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
            {
                assert_eq!(
                    imp::get_default(&r.registry_key(browser).unwrap()).unwrap(),
                    None
                );
                let key = r.registry_key(browser).unwrap();
                let parent = key.rsplit_once('\\').unwrap().0;
                assert!(!imp::key_exists(parent).unwrap(), "{parent} left");
            }
            // Again: nothing registered is not an error.
            r.unregister(browser).unwrap();
        }
        assert_eq!(world.files(), Vec::<PathBuf>::new(), "files left behind");
    }

    /// A browser that is not installed gets nothing: no folder, no key.
    #[test]
    fn a_browser_that_is_not_installed_is_left_alone() {
        let world = World::new(&[Browser::Chrome]);
        let r = &world.registrar;
        assert_eq!(
            r.register(Browser::Edge, &host(), &[CHROME_ID]).unwrap(),
            Outcome::NotInstalled
        );
        assert!(!r.is_registered(Browser::Edge).unwrap());
        assert_eq!(world.files(), Vec::<PathBuf>::new());
        #[cfg(windows)]
        assert!(
            !imp::key_exists(&format!(r"{}\{}", world.root, Browser::Edge.vendor_key())).unwrap()
        );
    }

    /// Another host registered in the same folder is kept.
    #[test]
    fn another_host_in_the_folder_is_kept() {
        let world = World::new(&[Browser::Chrome]);
        let r = &world.registrar;
        r.register(Browser::Chrome, &host(), &[CHROME_ID]).unwrap();
        let other = r
            .manifest_path(Browser::Chrome)
            .with_file_name("com.example.other.json");
        std::fs::write(&other, "{}").unwrap();
        r.unregister(Browser::Chrome).unwrap();
        assert!(other.is_file());
    }

    #[test]
    fn extension_ids_and_the_host_path_are_checked() {
        let world = World::new(&Browser::ALL);
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

    /// The real places, as the browsers document them.
    #[test]
    fn the_places_are_the_documented_ones() {
        let data = tempdir();
        let r = Registrar::for_current_user(&data).unwrap();
        let path = |b| r.manifest_path(b).to_string_lossy().replace('\\', "/");
        if cfg!(target_os = "macos") {
            assert!(path(Browser::Chrome).ends_with(&format!("Library/Application Support/Google/Chrome/NativeMessagingHosts/{NATIVE_HOST_NAME}.json")));
            assert!(path(Browser::Edge).ends_with(&format!("Library/Application Support/Microsoft Edge/NativeMessagingHosts/{NATIVE_HOST_NAME}.json")));
            assert!(path(Browser::Firefox).ends_with(&format!(
                "Library/Application Support/Mozilla/NativeMessagingHosts/{NATIVE_HOST_NAME}.json"
            )));
        } else if cfg!(windows) {
            assert_eq!(
                r.registry_key(Browser::Chrome).unwrap(),
                format!(r"Software\Google\Chrome\NativeMessagingHosts\{NATIVE_HOST_NAME}")
            );
            assert_eq!(
                r.registry_key(Browser::Edge).unwrap(),
                format!(r"Software\Microsoft\Edge\NativeMessagingHosts\{NATIVE_HOST_NAME}")
            );
            assert_eq!(
                r.registry_key(Browser::Firefox).unwrap(),
                format!(r"Software\Mozilla\NativeMessagingHosts\{NATIVE_HOST_NAME}")
            );
        } else {
            assert!(path(Browser::Chrome).ends_with(&format!(
                ".config/google-chrome/NativeMessagingHosts/{NATIVE_HOST_NAME}.json"
            )));
            assert!(path(Browser::Chromium).ends_with(&format!(
                ".config/chromium/NativeMessagingHosts/{NATIVE_HOST_NAME}.json"
            )));
            assert!(path(Browser::Edge).ends_with(&format!(
                ".config/microsoft-edge/NativeMessagingHosts/{NATIVE_HOST_NAME}.json"
            )));
            assert!(path(Browser::Firefox).ends_with(&format!(
                ".mozilla/native-messaging-hosts/{NATIVE_HOST_NAME}.json"
            )));
        }
        let _ = std::fs::remove_dir_all(data);
    }
}
