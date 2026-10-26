//! Categories (M4): which folder a finished download goes to, chosen by
//! the site it comes from or by its file type.
//!
//! They live in `categories.json` in the data folder, apart from the
//! queues (owner's decision on the M4 plan, #6): categories are settings
//! the user edits, queues are the state of work. The file follows the
//! rules of the queues file (ADR-0007, `docs/CATEGORIES-FILE.md`): a
//! `schema_version`, a SHA-256 `checksum`, strict reading (a damaged,
//! oversized, linked or unknown file is refused and never overwritten),
//! safe writes readable by the user only, and `categories.lock` held by
//! every change from its read to its write.
//!
//! How a download finds its category ([`CategoriesFile::classify`]):
//! 1. the first category, in order, with a site that the URL's host is
//!    or ends with (`example.com` matches `files.example.com`);
//! 2. otherwise the first category, in order, with the file's extension
//!    (the part after the last dot, letters ignoring case);
//! 3. otherwise none: the download stays in the folder it was given.

use std::fs::{File, OpenOptions};
use std::io::{self, Read};
use std::path::{Path, PathBuf};

use serde::{Deserialize, Serialize};

use crate::state::{checksum_of, now_rfc3339, write_safely};

/// Value of the `format` field.
pub const FORMAT: &str = "zajil-categories";
/// The schema this version of Zajil reads and writes.
pub const SCHEMA_VERSION: u32 = 1;
/// The most categories, extensions or sites anyone needs; more is refused.
pub const MAX_CATEGORIES: usize = 64;
pub const MAX_RULES: usize = 200;
/// The longest id and name, in characters.
pub const MAX_ID_CHARS: usize = 32;
pub const MAX_NAME_CHARS: usize = 64;

const FILE_NAME: &str = "categories.json";
const LOCK_NAME: &str = "categories.lock";
const MAX_FILE_BYTES: u64 = 1024 * 1024;

/// Everything in `categories.json`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct CategoriesFile {
    /// Always [`FORMAT`].
    pub format: String,
    /// Always [`SCHEMA_VERSION`] when written by this version.
    pub schema_version: u32,
    /// In order: the first that matches wins.
    pub categories: Vec<Category>,
    /// RFC 3339 time in UTC.
    pub updated_at: String,
    /// `sha256:` and the hex digest of the file with this field empty.
    pub checksum: String,
}

/// One category.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Category {
    /// Stable id: `programs`, `video`, `music`, `documents` and `archives`
    /// for the built-in ones, or one the user chose. Lowercase ASCII
    /// letters, digits, `-` and `_`.
    pub id: String,
    /// The name the user gave it. `None` for a built-in category that was
    /// not renamed: interfaces show its translated name (key
    /// `category-<id>`), so it follows the interface's language.
    #[serde(default)]
    pub name: Option<String>,
    /// File extensions without the dot, lowercase.
    #[serde(default)]
    pub extensions: Vec<String>,
    /// Host names, lowercase; a host matches itself and its subdomains.
    #[serde(default)]
    pub sites: Vec<String>,
    /// Absolute folder its downloads go to. `None`: the folder the
    /// download was given (nothing moves until the user picks one).
    #[serde(default)]
    pub folder: Option<PathBuf>,
}

/// What made [`CategoriesFile::classify_why`] choose a category.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Matched {
    /// One of its sites (as written in the category).
    Site(String),
    /// The file's extension (lowercase, without the dot).
    Extension(String),
}

/// Why the categories file could not be used or changed.
#[derive(Debug, thiserror::Error)]
pub enum CategoryError {
    #[error("could not read or write the categories file: {0}")]
    Io(#[from] io::Error),
    #[error("{} is not a Zajil categories file", .0.display())]
    NotACategoriesFile(PathBuf),
    #[error(
        "{} uses schema version {version}, which this version of Zajil cannot read",
        path.display()
    )]
    UnsupportedVersion { path: PathBuf, version: u64 },
    #[error(
        "the categories file {} is damaged ({reason}); Zajil leaves it as it is, so nothing in \
         it is lost: move it away to start again with the built-in categories",
        path.display()
    )]
    Corrupt { path: PathBuf, reason: String },
    #[error(
        "refusing to use {}: it is a symbolic link, a hard link or not a regular file",
        .0.display()
    )]
    NotRegularFile(PathBuf),
    #[error("there is no category {0:?}")]
    NoSuchCategory(String),
    #[error("there is already a category {0:?}")]
    Exists(String),
    #[error("{0}")]
    Invalid(String),
    #[error(
        "this change would make the categories file {bytes} bytes long, more than the \
         {MAX_FILE_BYTES} bytes Zajil reads back; nothing was changed"
    )]
    TooLarge { bytes: usize },
}

impl Default for CategoriesFile {
    fn default() -> Self {
        Self::built_in()
    }
}

impl CategoriesFile {
    /// The five built-in categories, with no folders of their own yet.
    pub fn built_in() -> Self {
        let cat = |id: &str, extensions: &[&str]| Category {
            id: id.to_owned(),
            name: None,
            extensions: extensions.iter().map(|e| (*e).to_owned()).collect(),
            sites: Vec::new(),
            folder: None,
        };
        CategoriesFile {
            format: FORMAT.to_owned(),
            schema_version: SCHEMA_VERSION,
            categories: vec![
                cat(
                    "programs",
                    &[
                        "exe", "msi", "msix", "dmg", "pkg", "deb", "rpm", "appimage", "apk",
                        "flatpak", "iso", "img",
                    ],
                ),
                cat(
                    "video",
                    &[
                        "mp4", "mkv", "avi", "mov", "webm", "wmv", "flv", "m4v", "mpg", "mpeg",
                    ],
                ),
                cat(
                    "music",
                    &["mp3", "flac", "m4a", "aac", "ogg", "opus", "wav", "wma"],
                ),
                cat(
                    "documents",
                    &[
                        "pdf", "doc", "docx", "xls", "xlsx", "ppt", "pptx", "odt", "ods", "odp",
                        "epub", "txt", "rtf", "csv",
                    ],
                ),
                cat(
                    "archives",
                    &["zip", "rar", "7z", "tar", "gz", "tgz", "bz2", "xz", "zst"],
                ),
            ],
            updated_at: now_rfc3339(),
            checksum: String::new(),
        }
    }

    pub fn get(&self, id: &str) -> Option<&Category> {
        self.categories.iter().find(|c| c.id == id)
    }

    pub fn get_mut(&mut self, id: &str) -> Result<&mut Category, CategoryError> {
        self.categories
            .iter_mut()
            .find(|c| c.id == id)
            .ok_or_else(|| CategoryError::NoSuchCategory(id.to_owned()))
    }

    /// The category of a download named `file_name` from `host` (the host
    /// of the final URL, after redirects), if any. See the module docs.
    pub fn classify(&self, file_name: &str, host: Option<&str>) -> Option<&Category> {
        self.classify_why(file_name, host)
            .map(|(category, _)| category)
    }

    /// [`classify`](Self::classify), and what matched: a site of the
    /// category, or the file's extension (the add dialog says which).
    pub fn classify_why(
        &self,
        file_name: &str,
        host: Option<&str>,
    ) -> Option<(&Category, Matched)> {
        if let Some(host) = host {
            let host = host.trim_end_matches('.').to_ascii_lowercase();
            for category in &self.categories {
                let site = category.sites.iter().find(|site| {
                    host == **site
                        || host
                            .strip_suffix(site.as_str())
                            .is_some_and(|rest| rest.ends_with('.'))
                });
                if let Some(site) = site {
                    return Some((category, Matched::Site(site.clone())));
                }
            }
        }
        let (_, ext) = file_name.rsplit_once('.')?;
        let ext = ext.to_lowercase();
        let category = self
            .categories
            .iter()
            .find(|c| c.extensions.contains(&ext))?;
        Some((category, Matched::Extension(ext)))
    }

    /// Where a download named `file_name` from `host` goes: its category's
    /// folder, or `given` when it has no category or the category has no
    /// folder.
    pub fn folder_for(&self, file_name: &str, host: Option<&str>, given: &Path) -> PathBuf {
        self.classify(file_name, host)
            .and_then(|c| c.folder.clone())
            .unwrap_or_else(|| given.to_owned())
    }

    /// Adds an empty category at the end.
    pub fn add(&mut self, id: &str, name: &str) -> Result<(), CategoryError> {
        check_id(id)?;
        check_name(name)?;
        if self.get(id).is_some() {
            return Err(CategoryError::Exists(id.to_owned()));
        }
        if self.categories.len() >= MAX_CATEGORIES {
            return Err(CategoryError::Invalid(format!(
                "there can be at most {MAX_CATEGORIES} categories"
            )));
        }
        self.categories.push(Category {
            id: id.to_owned(),
            name: Some(name.to_owned()),
            extensions: Vec::new(),
            sites: Vec::new(),
            folder: None,
        });
        Ok(())
    }

    pub fn remove(&mut self, id: &str) -> Result<Category, CategoryError> {
        let index = self
            .categories
            .iter()
            .position(|c| c.id == id)
            .ok_or_else(|| CategoryError::NoSuchCategory(id.to_owned()))?;
        Ok(self.categories.remove(index))
    }

    /// Rejects values that cannot be a real categories file even when the
    /// checksum matches.
    pub fn check_consistency(&self) -> Result<(), String> {
        if self.categories.len() > MAX_CATEGORIES {
            return Err(format!("more than {MAX_CATEGORIES} categories"));
        }
        let mut ids = std::collections::HashSet::new();
        for c in &self.categories {
            check_id(&c.id).map_err(|e| e.to_string())?;
            if !ids.insert(&c.id) {
                return Err(format!("the category {:?} appears twice", c.id));
            }
            if let Some(name) = &c.name {
                check_name(name).map_err(|e| e.to_string())?;
            }
            if c.extensions.len() > MAX_RULES || c.sites.len() > MAX_RULES {
                return Err(format!("category {:?} has too many rules", c.id));
            }
            for e in &c.extensions {
                check_extension(e).map_err(|e| e.to_string())?;
            }
            for s in &c.sites {
                check_site(s).map_err(|e| e.to_string())?;
            }
            if let Some(folder) = &c.folder {
                check_folder(folder).map_err(|e| e.to_string())?;
            }
        }
        Ok(())
    }
}

/// File types that can run code when opened (security review #84):
/// programs and installers, scripts, Java, macOS images and packages,
/// shortcuts and system settings files, Office files that can hold macros,
/// and disk images, which hand files inside them on without the mark of a
/// file from the internet.
#[rustfmt::skip]
pub const RUNS_CODE: &[&str] = &[
    // Windows programs, installers, scripts and what runs them
    "exe", "msi", "msix", "msixbundle", "msp", "msu", "appx", "appxbundle", "com", "pif", "scr",
    "cpl", "bat", "cmd", "ps1", "psm1", "psd1", "ps1xml", "vbs", "vbe", "js", "jse", "wsf",
    "wsh", "wsc", "sct", "hta", "chm", "msc", "inf", "reg", "lnk", "url", "scf", "cab",
    "diagcab", "library-ms", "appref-ms", "application", "settingcontent-ms", "website", "mht",
    "mhtml",
    // Java, macOS and Unix
    "jar", "jnlp", "dmg", "pkg", "mpkg", "app", "command", "terminal", "tool", "workflow", "scpt",
    "fileloc", "sh", "run", "desktop", "appimage", "deb", "rpm",
    // Office and other documents that can hold macros or run code
    "doc", "dot", "docm", "dotm", "wll", "xls", "xlt", "xla", "xll", "xlsm", "xltm", "xlsb",
    "xlam", "ppt", "pot", "pps", "ppa", "pptm", "potm", "ppsm", "ppam", "sldm", "one", "rtf",
    "mdb", "accdb", "accde", "pub", "vsd", "vsdm", "iqy",
    // Disk images
    "iso", "img", "vhd", "vhdx",
];

/// Whether a file named `name` can run code when opened (its extension is
/// in [`RUNS_CODE`], whatever its case).
pub fn runs_code(name: &str) -> bool {
    // Windows drops trailing dots and spaces: `setup.exe.` is `setup.exe`.
    name.trim_end_matches(['.', ' '])
        .rsplit_once('.')
        .map(|(_, ext)| ext.to_ascii_lowercase())
        .is_some_and(|ext| RUNS_CODE.contains(&ext.as_str()))
}

/// `programs`, `my-books`: 1 to [`MAX_ID_CHARS`] lowercase ASCII letters,
/// digits, `-` or `_`.
pub fn check_id(id: &str) -> Result<(), CategoryError> {
    let ok = (1..=MAX_ID_CHARS).contains(&id.len())
        && id
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'_');
    if ok {
        Ok(())
    } else {
        Err(CategoryError::Invalid(format!(
            "invalid category id {id:?}: use 1 to {MAX_ID_CHARS} lowercase letters, digits, `-` \
             or `_`"
        )))
    }
}

/// A name the user gives: 1 to [`MAX_NAME_CHARS`] characters, with no
/// control or invisible formatting characters (no U+202E tricks).
pub fn check_name(name: &str) -> Result<(), CategoryError> {
    let count = name.chars().count();
    let ok = (1..=MAX_NAME_CHARS).contains(&count)
        && name.trim() == name
        && !has_hidden(name)
        // Not only joiners and variation selectors, which show nothing.
        && name.chars().any(|c| !is_part_of_text(c));
    if ok {
        Ok(())
    } else {
        Err(CategoryError::Invalid(format!(
            "invalid category name {name:?}: use 1 to {MAX_NAME_CHARS} visible characters"
        )))
    }
}

/// `zip`, `7z`: 1 to 16 lowercase ASCII letters or digits.
pub fn check_extension(ext: &str) -> Result<(), CategoryError> {
    let ok = (1..=16).contains(&ext.len())
        && ext
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit());
    if ok {
        Ok(())
    } else {
        Err(CategoryError::Invalid(format!(
            "invalid extension {ext:?}: use 1 to 16 lowercase letters or digits, without the dot"
        )))
    }
}

/// `example.com`: a lowercase host name (letters, digits, `-` and dots,
/// punycode for other scripts), at most 253 characters.
pub fn check_site(site: &str) -> Result<(), CategoryError> {
    let ok = (1..=253).contains(&site.len())
        && !site.starts_with('.')
        && !site.ends_with('.')
        && !site.contains("..")
        && site
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'-' || b == b'.');
    if ok {
        Ok(())
    } else {
        Err(CategoryError::Invalid(format!(
            "invalid site {site:?}: use a host name such as example.com"
        )))
    }
}

/// A category's folder must be absolute, without `..`.
pub fn check_folder(folder: &Path) -> Result<(), CategoryError> {
    let ok = folder.is_absolute()
        && !folder
            .components()
            .any(|c| matches!(c, std::path::Component::ParentDir))
        && folder.to_str().is_some_and(|text| !has_hidden(text));
    if ok {
        Ok(())
    } else {
        Err(CategoryError::Invalid(format!(
            "invalid folder {}: use an absolute path without `..`, as valid text without control \
             or invisible characters",
            folder.display()
        )))
    }
}

/// A site as the user typed it, made comparable: lowercase, and other
/// scripts in punycode (`مثال.مصر` becomes `xn--mgbh0fb.xn--wgbh1c`).
pub fn normalize_site(input: &str) -> Result<String, CategoryError> {
    let input = input.trim().trim_end_matches('.');
    // A host name only: no user, port, path, query or fragment.
    if input.is_empty() || input.contains(['/', '\\', '@', ':', '?', '#', ' ']) {
        return Err(CategoryError::Invalid(format!(
            "invalid site {input:?}: use a host name such as example.com"
        )));
    }
    let url = reqwest::Url::parse(&format!("http://{input}/"))
        .map_err(|_| CategoryError::Invalid(format!("invalid site {input:?}")))?;
    let host = url
        .host_str()
        .filter(|h| url.path() == "/" && !h.is_empty())
        .ok_or_else(|| CategoryError::Invalid(format!("invalid site {input:?}")))?
        .to_owned();
    check_site(&host)?;
    Ok(host)
}

/// An extension as the user typed it (`.ZIP`), made comparable (`zip`).
pub fn normalize_extension(input: &str) -> Result<String, CategoryError> {
    let ext = input.trim().trim_start_matches('.').to_lowercase();
    check_extension(&ext)?;
    Ok(ext)
}

/// `text` with what would drive a terminal or reorder text written as
/// escapes (`\u{1b}`): for messages that quote a value from a file, such as
/// serde's (review of #41, round 5: an import file's escape codes reached
/// the terminal through them).
pub(crate) fn printable(text: &str) -> String {
    text.chars()
        .map(|c| {
            if is_display_control(c) {
                c.escape_unicode().to_string()
            } else {
                c.to_string()
            }
        })
        .collect()
}

/// A character that changes how the text around it is shown: a control
/// (escape codes drive a terminal), a direction mark that reorders text,
/// such as U+202E, or a line or paragraph separator. Never in a URL a
/// queue holds (review of #41, round 4).
pub(crate) fn is_display_control(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{061C}'
                | '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2028}'
                | '\u{2029}'
        )
}

/// What a URL a queue holds may never contain: the display controls,
/// except tab and line breaks, which the URL parser removes anyway (review
/// of #41, round 5). The same rule for `QueuesFile::add` and imports, so
/// Zajil imports what it exports.
pub(crate) fn is_unsafe_in_url(c: char) -> bool {
    is_display_control(c) && !matches!(c, '\t' | '\n' | '\r')
}

/// Joiners and variation selectors: invisible, but part of real text (the
/// ZWNJ of Persian «فیلم‌ها», the U+FE0F of «❤️», an ideographic variant),
/// and unable to reorder it (review of #41, round 4).
fn is_part_of_text(c: char) -> bool {
    matches!(
        c,
        '\u{200C}' | '\u{200D}' | '\u{FE00}'..='\u{FE0F}' | '\u{E0100}'..='\u{E01EF}'
            | '\u{180B}'..='\u{180D}' | '\u{180F}'
    )
}

/// Whether `text` holds a hidden character, or a joiner or variation
/// selector out of the place it has in real text (review of #41, round 5:
/// `Vi\u{200C}deos` looked like `Videos`). A joiner must stand between two
/// characters that are not ASCII (Persian «فیلم‌ها», Sinhala, emoji
/// sequences); a variation selector must follow a character that is not
/// ASCII, or a keycap base (`0`–`9`, `#`, `*`, as in «1️⃣»).
pub(crate) fn has_hidden(text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    let base = |c: Option<&char>| c.is_some_and(|c| !c.is_ascii() && !c.is_whitespace());
    chars.iter().enumerate().any(|(i, &c)| {
        let before = i.checked_sub(1).and_then(|j| chars.get(j));
        let after = chars.get(i + 1);
        match c {
            '\u{200C}' | '\u{200D}' => {
                !(base(before) && !matches!(before, Some('\u{200C}' | '\u{200D}')) && base(after))
            }
            c if is_part_of_text(c) => {
                let keycap = before.is_some_and(|b| b.is_ascii_digit() || matches!(b, '#' | '*'));
                !(keycap || base(before) && !before.is_some_and(|b| is_part_of_text(*b)))
            }
            c => is_hidden_char(c),
        }
    })
}

/// A character that is invisible and could disguise a name or a folder:
/// the display controls above, formatting characters, and every
/// default-ignorable code point, such as the Hangul filler U+3164 or the
/// combining grapheme joiner U+034F (review of #41, round 3), except the
/// joiners and variation selectors that are part of real text (round 4).
pub(crate) fn is_hidden_char(c: char) -> bool {
    use icu_properties::props::{DefaultIgnorableCodePoint, GeneralCategory};
    use icu_properties::{CodePointMapData, CodePointSetData};
    if is_part_of_text(c) {
        return false;
    }
    is_display_control(c)
        || CodePointSetData::new::<DefaultIgnorableCodePoint>().contains(c)
        || matches!(
            CodePointMapData::<GeneralCategory>::new().get(c),
            GeneralCategory::Format
                | GeneralCategory::LineSeparator
                | GeneralCategory::ParagraphSeparator
        )
}

/// The categories file in one folder, and its lock.
#[derive(Debug, Clone)]
pub struct CategoryStore {
    dir: PathBuf,
}

impl CategoryStore {
    /// The store in `dir` (normally `AppPaths::data_dir`). Nothing is
    /// created until the first change.
    pub fn new(dir: impl Into<PathBuf>) -> Self {
        CategoryStore { dir: dir.into() }
    }

    /// `categories.json`.
    pub fn path(&self) -> PathBuf {
        self.dir.join(FILE_NAME)
    }

    /// Reads and fully checks the file; a missing file is
    /// [`CategoriesFile::built_in`]. Blocking.
    pub fn load(&self) -> Result<CategoriesFile, CategoryError> {
        let path = self.path();
        let file = match zajil_platform::fs::open_regular_nofollow(&path, false) {
            Ok(file) => file,
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                return Ok(CategoriesFile::built_in());
            }
            Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                return Err(CategoryError::NotRegularFile(path));
            }
            Err(err) => return Err(err.into()),
        };
        let mut bytes = Vec::new();
        file.take(MAX_FILE_BYTES + 1).read_to_end(&mut bytes)?;
        let corrupt = |reason: String| CategoryError::Corrupt {
            path: path.clone(),
            reason,
        };
        if bytes.len() as u64 > MAX_FILE_BYTES {
            return Err(corrupt("the file is too large".into()));
        }
        let value: serde_json::Value =
            serde_json::from_slice(&bytes).map_err(|e| corrupt(printable(&e.to_string())))?;
        if value.get("format").and_then(|v| v.as_str()) != Some(FORMAT) {
            return Err(CategoryError::NotACategoriesFile(path));
        }
        match value.get("schema_version").and_then(|v| v.as_u64()) {
            Some(v) if v == u64::from(SCHEMA_VERSION) => {}
            Some(version) => return Err(CategoryError::UnsupportedVersion { path, version }),
            None => return Err(corrupt("missing schema_version".into())),
        }
        let Some(stored) = value.get("checksum").and_then(|v| v.as_str()) else {
            return Err(corrupt("missing checksum".into()));
        };
        if stored != checksum_of(&value) {
            return Err(corrupt("its checksum does not match".into()));
        }
        let file: CategoriesFile =
            serde_json::from_value(value).map_err(|e| corrupt(printable(&e.to_string())))?;
        file.check_consistency().map_err(corrupt)?;
        Ok(file)
    }

    /// Changes the file with `change`, holding `categories.lock` from the
    /// read to the end of the write. A damaged file is never overwritten.
    /// Blocking.
    pub fn update<T>(
        &self,
        change: impl FnOnce(&mut CategoriesFile) -> Result<T, CategoryError>,
    ) -> Result<T, CategoryError> {
        let _lock = self.lock()?;
        let mut file = self.load()?;
        let result = change(&mut file)?;
        file.check_consistency().map_err(CategoryError::Invalid)?;
        file.format = FORMAT.to_owned();
        file.schema_version = SCHEMA_VERSION;
        file.updated_at = now_rfc3339();
        file.checksum = String::new();
        file.checksum = checksum_of(&serde_json::to_value(&file).map_err(io::Error::other)?);
        let mut json = serde_json::to_vec_pretty(&file).map_err(io::Error::other)?;
        json.push(b'\n');
        // Never write a file that `load` would refuse (review of #41).
        if json.len() as u64 > MAX_FILE_BYTES {
            return Err(CategoryError::TooLarge { bytes: json.len() });
        }
        write_safely(&self.path(), &json)?;
        Ok(result)
    }

    /// Opens (creating if needed) `categories.lock` and waits for its lock,
    /// never through a link (AGENTS.md §10.1).
    fn lock(&self) -> Result<File, CategoryError> {
        std::fs::create_dir_all(&self.dir)?;
        let path = self.dir.join(LOCK_NAME);
        let file = loop {
            match zajil_platform::fs::open_regular_nofollow(&path, true) {
                Ok(file) => break file,
                Err(err) if zajil_platform::fs::is_not_regular_file(&err) => {
                    return Err(CategoryError::NotRegularFile(path));
                }
                Err(err) if err.kind() == io::ErrorKind::NotFound => {}
                Err(err) => return Err(err.into()),
            }
            match OpenOptions::new().write(true).create_new(true).open(&path) {
                Ok(file) => break file,
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(err) => return Err(err.into()),
            }
        };
        file.lock()?;
        Ok(file)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Security review #84: what the owner listed, any case, and a name
    /// that only looks like one (`setup.exe.zip`, a trailing dot or space,
    /// which Windows drops) seen for what it is.
    #[test]
    fn files_that_can_run_code_are_known() {
        for name in [
            "setup.exe",
            "Setup.EXE",
            "a.msi",
            "run.bat",
            "run.cmd",
            "x.ps1",
            "s.scr",
            "a.js",
            "a.vbs",
            "app.jar",
            "Zajil.dmg",
            "a.pkg",
            "Zajil.app",
            "install.sh",
            "report.docm",
            "sheet.xlsm",
            "old.doc",
            "slides.pptm",
            "image.iso",
            "setup.exe.",
            "setup.exe ",
        ] {
            assert!(runs_code(name), "{name}");
        }
        for name in [
            "a.zip",
            "setup.exe.zip",
            "film.mp4",
            "notes.txt",
            "report.docx",
            "exe",
            "",
        ] {
            assert!(!runs_code(name), "{name}");
        }
    }

    /// Owner's decision on #152: these run code (or open what does) when
    /// opened; a web page (`.html`, `.xhtml`, `.xml`) does not count: the
    /// mark of a file from the internet is enough, and the browser keeps
    /// it apart.
    #[test]
    fn the_owner_s_list_of_152_runs_code_and_web_pages_do_not() {
        for ext in [
            "hta",
            "mht",
            "mhtml",
            "url",
            "website",
            "lnk",
            "scf",
            "library-ms",
            "settingcontent-ms",
            "appref-ms",
            "jnlp",
            "xll",
            "iqy",
        ] {
            assert!(runs_code(&format!("a.{ext}")), "{ext}");
            assert!(runs_code(&format!("A.{}", ext.to_uppercase())), "{ext}");
        }
        for name in ["view.html", "page.xhtml", "feed.xml", "page.htm"] {
            assert!(!runs_code(name), "{name}");
        }
    }

    /// Review of #48: the add dialog says what matched, a site or the
    /// extension, and a site wins as in `classify`.
    #[test]
    fn classify_why_says_what_matched() {
        let mut c = CategoriesFile::built_in();
        c.categories[0].sites.push("example.com".into());
        let first = c.categories[0].id.clone();
        let (category, matched) = c
            .classify_why("photos.ZIP", Some("dl.example.com"))
            .unwrap();
        assert_eq!(
            (category.id.as_str(), matched),
            (first.as_str(), Matched::Site("example.com".into()))
        );
        let (category, matched) = c.classify_why("photos.ZIP", Some("other.net")).unwrap();
        assert_eq!(
            (category.id.as_str(), matched),
            ("archives", Matched::Extension("zip".into()))
        );
        assert!(c.classify_why("README", Some("other.net")).is_none());
    }

    #[test]
    fn a_zip_file_is_an_archive_and_extensions_ignore_case() {
        let c = CategoriesFile::built_in();
        assert_eq!(c.classify("photos.zip", None).unwrap().id, "archives");
        assert_eq!(c.classify("PHOTOS.ZIP", None).unwrap().id, "archives");
        assert_eq!(c.classify("x.tar.gz", None).unwrap().id, "archives");
        assert_eq!(c.classify("محاضرة.mp4", None).unwrap().id, "video");
        assert!(c.classify("README", None).is_none());
        assert!(c.classify("notes.unknown", None).is_none());
    }

    #[test]
    fn a_site_wins_over_the_extension_and_matches_subdomains_only() {
        let mut c = CategoriesFile::built_in();
        c.add("nas", "النسخ الاحتياطية").unwrap();
        c.get_mut("nas").unwrap().sites.push("example.net".into());
        assert_eq!(
            c.classify("a.zip", Some("files.example.net")).unwrap().id,
            "nas"
        );
        assert_eq!(c.classify("a.zip", Some("EXAMPLE.NET.")).unwrap().id, "nas");
        // Not a subdomain, only the same ending.
        assert_eq!(
            c.classify("a.zip", Some("badexample.net")).unwrap().id,
            "archives"
        );
    }

    #[test]
    fn names_sites_extensions_and_folders_are_checked() {
        assert!(check_name("a\u{202e}b").is_err());
        assert!(check_name(" a").is_err());
        assert!(check_name("مستندات العمل").is_ok());
        assert!(check_id("Programs").is_err());
        assert!(check_extension(".zip").is_err());
        assert_eq!(normalize_extension(" .ZIP ").unwrap(), "zip");
        assert_eq!(
            normalize_site("Files.Example.NET.").unwrap(),
            "files.example.net"
        );
        assert!(normalize_site("example.com/path").is_err());
        assert!(normalize_site("user@example.com").is_err());
        assert!(normalize_site("مثال.مصر").unwrap().starts_with("xn--"));
        assert!(check_folder(Path::new("relative/dir")).is_err());
        let hidden = std::env::temp_dir().join("a\u{1b}[31mb");
        assert!(check_folder(&hidden).is_err());
        let spoof = std::env::temp_dir().join("x\u{202e}y");
        assert!(check_folder(&spoof).is_err());
        let dotdot = std::env::temp_dir().join("..").join("x");
        assert!(check_folder(&dotdot).is_err());
    }
}
