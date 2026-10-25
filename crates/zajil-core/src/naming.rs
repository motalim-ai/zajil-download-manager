//! Choosing the name of a downloaded file.
//!
//! The name comes from `Content-Disposition` (RFC 6266, with RFC 8187
//! `filename*`), otherwise from the last segment of the final URL. Either way
//! it is untrusted and goes through [`sanitize_file_name`].

use std::fs::{File, OpenOptions};
use std::io;
use std::path::{Path, PathBuf};

use percent_encoding::percent_decode_str;

use crate::sanitize::{ConfineError, MAX_NAME_BYTES, confine, sanitize_file_name};
use crate::state::{part_path, state_path};

/// How many numbered alternatives (`name (1).ext` …) we try.
const MAX_ALTERNATIVES: u32 = 1000;

/// The safe file name for a response.
pub fn suggested_name(content_disposition: Option<&[u8]>, final_url: &reqwest::Url) -> String {
    let from_header = content_disposition.and_then(filename_from_content_disposition);
    let from_url = || {
        final_url
            .path_segments()
            .and_then(|mut segments| segments.next_back())
            .filter(|segment| !segment.is_empty())
            .map(|segment| percent_decode_str(segment).decode_utf8_lossy().into_owned())
    };
    sanitize_file_name(&from_header.or_else(from_url).unwrap_or_default())
}

/// File extensions for common media types, for a name that has none
/// (an address such as `…/view`). Never a type that can run code (none of
/// these is in [`crate::category::RUNS_CODE`]): a server must not turn
/// `view` into `view.exe` by its `Content-Type`.
#[rustfmt::skip]
const EXTENSIONS_BY_TYPE: &[(&str, &str)] = &[
    ("text/html", "html"), ("application/xhtml+xml", "xhtml"), ("text/plain", "txt"),
    ("text/csv", "csv"), ("application/json", "json"), ("application/xml", "xml"),
    ("text/xml", "xml"), ("application/pdf", "pdf"), ("application/epub+zip", "epub"),
    ("application/zip", "zip"), ("application/x-7z-compressed", "7z"),
    ("application/vnd.rar", "rar"), ("application/x-rar-compressed", "rar"),
    ("application/gzip", "gz"), ("application/x-gzip", "gz"), ("application/x-tar", "tar"),
    ("application/x-xz", "xz"), ("image/jpeg", "jpg"), ("image/png", "png"),
    ("image/gif", "gif"), ("image/webp", "webp"), ("image/avif", "avif"),
    ("video/mp4", "mp4"), ("video/webm", "webm"), ("video/x-matroska", "mkv"),
    ("video/quicktime", "mov"), ("audio/mpeg", "mp3"), ("audio/mp4", "m4a"),
    ("audio/ogg", "ogg"), ("audio/flac", "flac"), ("audio/wav", "wav"), ("audio/x-wav", "wav"),
];

/// The extension of the media type `content_type` (a `Content-Type` value,
/// its parameters ignored), if it is one of the common ones above.
pub fn extension_for_type(content_type: &str) -> Option<&'static str> {
    let media_type = content_type.split(';').next()?.trim().to_ascii_lowercase();
    EXTENSIONS_BY_TYPE
        .iter()
        .find(|(t, _)| *t == media_type)
        .map(|(_, ext)| *ext)
}

/// `name` with the extension of `content_type` when it has none: `view`
/// sent as `text/html` is `view.html`. Cleaned again, so it stays within
/// [`MAX_NAME_BYTES`].
pub fn with_type_extension(name: String, content_type: Option<&str>) -> String {
    let has_extension = name
        .rsplit_once('.')
        .is_some_and(|(stem, ext)| !stem.is_empty() && !ext.is_empty());
    match content_type.and_then(extension_for_type) {
        Some(ext) if !has_extension => sanitize_file_name(&format!("{name}.{ext}")),
        _ => name,
    }
}

/// Extracts the file name from a `Content-Disposition` value. `filename*`
/// wins over `filename` (RFC 6266 §4.3).
pub fn filename_from_content_disposition(value: &[u8]) -> Option<String> {
    // Servers sometimes send raw UTF-8; accept it rather than fail.
    let value = String::from_utf8_lossy(value);
    let mut plain = None;
    let mut extended = None;
    for param in split_params(&value).into_iter().skip(1) {
        let Some((name, raw)) = param.split_once('=') else {
            continue;
        };
        match name.trim().to_ascii_lowercase().as_str() {
            // An empty value counts as absent, so it cannot hide the other.
            "filename*" => {
                extended =
                    extended.or_else(|| decode_ext_value(raw.trim()).filter(|s| !s.is_empty()))
            }
            "filename" => {
                plain = plain.or_else(|| Some(unquote(raw.trim())).filter(|s| !s.is_empty()))
            }
            _ => {}
        }
    }
    extended.or(plain)
}

/// Splits on `;` outside quoted strings.
fn split_params(value: &str) -> Vec<String> {
    let mut params = Vec::new();
    let mut current = String::new();
    let mut quoted = false;
    let mut escaped = false;
    for c in value.chars() {
        match c {
            _ if escaped => escaped = false,
            '\\' if quoted => escaped = true,
            '"' => quoted = !quoted,
            ';' if !quoted => {
                params.push(std::mem::take(&mut current));
                continue;
            }
            _ => {}
        }
        current.push(c);
    }
    params.push(current);
    params
}

/// A token, or a quoted-string with `\"` and `\\` unescaped. Any other
/// backslash is kept: many servers send Windows paths unescaped (RFC 6266
/// appendix D), and the path is then stripped like any other.
fn unquote(raw: &str) -> String {
    let Some(inner) = raw.strip_prefix('"') else {
        return raw.to_owned();
    };
    let inner = inner.strip_suffix('"').unwrap_or(inner);
    let mut out = String::with_capacity(inner.len());
    let mut chars = inner.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\\'
            && let Some(&next @ ('"' | '\\')) = chars.peek()
        {
            out.push(next);
            chars.next();
        } else {
            out.push(c);
        }
    }
    out
}

/// RFC 8187 `charset'language'percent-encoded`, for UTF-8 and ISO-8859-1.
fn decode_ext_value(raw: &str) -> Option<String> {
    let mut parts = raw.splitn(3, '\'');
    let charset = parts.next()?.trim().to_ascii_lowercase();
    let _language = parts.next()?;
    let encoded = parts.next()?;
    let bytes: Vec<u8> = percent_decode_str(encoded).collect();
    match charset.as_str() {
        "utf-8" => String::from_utf8(bytes).ok(),
        "iso-8859-1" => Some(bytes.into_iter().map(char::from).collect()),
        _ => None,
    }
}

/// A free target in `dir`: the final path and its newly created, empty
/// `.zajil-part` file. A name is free when none of the final file, its part
/// file or its state file exists; otherwise `name (1).ext`, `name (2).ext`
/// and so on are tried.
pub fn reserve_target(dir: &Path, name: &str) -> Result<(PathBuf, File), ReserveError> {
    for candidate in candidates(name) {
        let final_path = confine(dir, &candidate)?;
        if final_path.symlink_metadata().is_ok()
            || state_path(&final_path).symlink_metadata().is_ok()
        {
            continue;
        }
        match OpenOptions::new()
            .read(true)
            .write(true)
            .create_new(true)
            .open(part_path(&final_path))
        {
            Ok(file) => return Ok((final_path, file)),
            Err(err) if err.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(err) => return Err(err.into()),
        }
    }
    Err(ReserveError::NoFreeName(name.to_owned()))
}

/// `name`, then `name (1).ext`, `name (2).ext` and so on.
pub(crate) fn candidates(name: &str) -> impl Iterator<Item = String> + '_ {
    (0..MAX_ALTERNATIVES).map(move |n| {
        if n == 0 {
            name.to_owned()
        } else {
            numbered(name, n)
        }
    })
}

/// Errors from [`reserve_target`].
#[derive(Debug, thiserror::Error)]
pub enum ReserveError {
    #[error(transparent)]
    Confine(#[from] ConfineError),
    #[error(transparent)]
    Io(#[from] io::Error),
    #[error("no free file name for {0:?} in the download folder")]
    NoFreeName(String),
}

/// `name (n).ext`, keeping the extension. The stem is cut first so the
/// result never exceeds [`MAX_NAME_BYTES`] (and so its `.zajil-part` and
/// `.zajil-state` names stay within the file-system limit), and the result
/// is a name [`sanitize_file_name`] leaves unchanged, which `resume`
/// requires.
fn numbered(name: &str, n: u32) -> String {
    let suffix = format!(" ({n})");
    let (stem, extension) = match name.rfind('.') {
        // An extension too long to keep is treated as part of the stem.
        Some(dot) if dot > 0 && name.len() - dot + suffix.len() < MAX_NAME_BYTES => {
            name.split_at(dot)
        }
        _ => (name, ""),
    };
    let budget = MAX_NAME_BYTES - suffix.len() - extension.len();
    let mut cut = stem.len().min(budget);
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    // Cutting can leave something sanitizing would change (a joiner or a
    // dot at the cut), so normalise once.
    let candidate = sanitize_file_name(&format!("{}{suffix}{extension}", &stem[..cut]));
    debug_assert!(candidate.len() <= MAX_NAME_BYTES, "{candidate:?}");
    debug_assert_eq!(sanitize_file_name(&candidate), candidate);
    candidate
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_name_without_an_extension_takes_its_type_s() {
        assert_eq!(
            with_type_extension("view".into(), Some("text/html; charset=utf-8")),
            "view.html"
        );
        assert_eq!(
            with_type_extension("download".into(), Some("application/PDF")),
            "download.pdf"
        );
        // An extension already there, or no known type: as it is.
        assert_eq!(
            with_type_extension("a.zip".into(), Some("text/html")),
            "a.zip"
        );
        assert_eq!(
            with_type_extension("view".into(), Some("application/octet-stream")),
            "view"
        );
        assert_eq!(with_type_extension("view".into(), None), "view");
        // Never a type that runs code.
        assert_eq!(
            with_type_extension("view".into(), Some("application/x-msdownload")),
            "view"
        );
        // At the limit, it stays within it, with its extension, and is a
        // name the engine accepts back as it is (AGENTS §10.7): Arabic
        // names of 197 to 200 bytes (an odd byte with a Latin letter).
        for bytes in 197..=MAX_NAME_BYTES {
            let long = format!("{}{}", "ع".repeat(bytes / 2), "x".repeat(bytes % 2));
            assert_eq!(long.len(), bytes);
            let named = with_type_extension(long, Some("text/html"));
            assert!(
                named.len() <= MAX_NAME_BYTES && named.ends_with(".html"),
                "{named}"
            );
            assert_eq!(sanitize_file_name(&named), named);
        }
        // A reserved name stays safe.
        assert_eq!(
            with_type_extension("_CON".into(), Some("text/plain")),
            "_CON.txt"
        );
    }

    #[test]
    fn no_type_gives_an_extension_that_runs_code() {
        for (_, ext) in EXTENSIONS_BY_TYPE {
            assert!(!crate::category::runs_code(&format!("a.{ext}")), "{ext}");
        }
    }

    fn cd(value: &str) -> Option<String> {
        filename_from_content_disposition(value.as_bytes())
    }

    fn url(s: &str) -> reqwest::Url {
        reqwest::Url::parse(s).unwrap()
    }

    #[test]
    fn reads_plain_and_quoted_filenames() {
        assert_eq!(cd("attachment; filename=a.zip").as_deref(), Some("a.zip"));
        assert_eq!(
            cd("attachment; filename=\"my file.zip\"").as_deref(),
            Some("my file.zip")
        );
        assert_eq!(
            cd("attachment; filename=\"a\\\"b;c.zip\"").as_deref(),
            Some("a\"b;c.zip")
        );
        assert_eq!(
            cd("attachment; filename=\"a\\\\b.zip\"").as_deref(),
            Some("a\\b.zip")
        );
        assert_eq!(cd("attachment").as_deref(), None);
        // Review of #15: an empty `filename*` must not hide `filename`.
        assert_eq!(
            cd("attachment; filename*=UTF-8''; filename=\"x.zip\"").as_deref(),
            Some("x.zip")
        );
        assert_eq!(
            cd("attachment; filename=\"\"; filename*=UTF-8''y.zip").as_deref(),
            Some("y.zip")
        );
        assert_eq!(cd("attachment; filename=\"\"").as_deref(), None);
    }

    #[test]
    fn prefers_the_extended_filename() {
        let value = "attachment; filename=\"fallback.pdf\"; filename*=UTF-8''%D8%AA%D9%82%D8%B1%D9%8A%D8%B1.pdf";
        assert_eq!(cd(value).as_deref(), Some("تقرير.pdf"));
        assert_eq!(
            cd("attachment; filename*=iso-8859-1'en'%E9t%E9.txt").as_deref(),
            Some("été.txt")
        );
        // An unknown charset falls back to `filename`.
        assert_eq!(
            cd("attachment; filename*=koi8-r''%C1; filename=x.txt").as_deref(),
            Some("x.txt")
        );
    }

    #[test]
    fn accepts_raw_utf8_from_misbehaving_servers() {
        let value = "attachment; filename=\"ملف.zip\"";
        assert_eq!(cd(value).as_deref(), Some("ملف.zip"));
    }

    #[test]
    fn falls_back_to_the_url() {
        assert_eq!(
            suggested_name(None, &url("http://h/dir/a%20b.zip?x=1")),
            "a b.zip"
        );
        assert_eq!(suggested_name(None, &url("http://h/")), "download");
        assert_eq!(
            suggested_name(None, &url("http://h/a/%2E%2E%2F%2E%2E%2Fx")),
            "x"
        );
    }

    #[test]
    fn keeps_unescaped_windows_paths_so_they_can_be_stripped() {
        let value = "attachment; filename=\"C:\\Users\\me\\Temp\\setup.exe\"";
        assert_eq!(
            suggested_name(Some(value.as_bytes()), &url("http://h/f")),
            "setup.exe"
        );
    }

    #[test]
    fn header_names_are_sanitized() {
        let name = suggested_name(
            Some(b"attachment; filename=\"../../etc/passwd\""),
            &url("http://h/f"),
        );
        assert_eq!(name, "passwd");
    }

    #[test]
    fn numbered_long_names_stay_within_the_limit_and_stay_clean() {
        // Review of #17: a 197–200 byte name became 204 bytes with " (1)",
        // and `resume` then refused the state file Zajil had written.
        let stems = ["a".repeat(250), "ملف".repeat(60), "x\u{200C}y".repeat(60)];
        for stem in &stems {
            for total in 190..=MAX_NAME_BYTES {
                for ext in ["", ".zip", ".tar.gz"] {
                    let raw = format!("{stem}{ext}");
                    let name = sanitize_file_name(&raw[..floor(&raw, total)]);
                    for n in [1, 9, 10, 999] {
                        let candidate = numbered(&name, n);
                        assert!(
                            candidate.len() <= MAX_NAME_BYTES,
                            "{} bytes",
                            candidate.len()
                        );
                        assert_eq!(sanitize_file_name(&candidate), candidate);
                        assert!(candidate.contains(&format!("({n})")), "{candidate:?}");
                        let state = format!("{candidate}.zajil-state.tmp");
                        assert!(state.len() <= 255, "{} bytes", state.len());
                    }
                }
            }
        }
    }

    /// The largest char boundary of `s` at or below `max`.
    fn floor(s: &str, max: usize) -> usize {
        let mut cut = max.min(s.len());
        while !s.is_char_boundary(cut) {
            cut -= 1;
        }
        cut
    }

    #[test]
    fn reserves_a_numbered_name_for_a_name_at_the_limit() {
        let dir = tempfile::tempdir().unwrap();
        let name = format!("{}.zip", "a".repeat(194)); // 198 bytes
        std::fs::write(dir.path().join(&name), b"old").unwrap();
        let (path, _part) = reserve_target(dir.path(), &name).unwrap();
        let taken = path.file_name().unwrap().to_str().unwrap();
        assert!(taken.len() <= MAX_NAME_BYTES, "{}", taken.len());
        assert!(taken.ends_with(" (1).zip"), "{taken}");
        assert_eq!(sanitize_file_name(taken), taken);
    }

    #[test]
    fn numbers_alternatives() {
        assert_eq!(numbered("a.tar.gz", 2), "a.tar (2).gz");
        assert_eq!(numbered("README", 1), "README (1)");
    }

    #[test]
    fn reserves_a_free_name() {
        let dir = tempfile::tempdir().unwrap();
        std::fs::write(dir.path().join("a.zip"), b"old").unwrap();
        std::fs::write(dir.path().join("a (1).zip.zajil-state"), b"{}").unwrap();
        let (path, _part) = reserve_target(dir.path(), "a.zip").unwrap();
        assert_eq!(path.file_name().unwrap(), "a (2).zip");
        assert!(dir.path().join("a (2).zip.zajil-part").exists());
        // The part file now blocks the same name.
        let (next, _) = reserve_target(dir.path(), "a.zip").unwrap();
        assert_eq!(next.file_name().unwrap(), "a (3).zip");
    }
}
