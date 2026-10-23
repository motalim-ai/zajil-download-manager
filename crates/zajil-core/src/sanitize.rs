//! File names from the network are untrusted.
//!
//! [`sanitize_file_name`] turns any string into one safe file name, using the
//! same rules on every OS so that a download is portable between systems.
//! [`confine`] then joins it to the download folder and refuses anything that
//! would land outside that folder.
//!
//! Sources: RFC 6266 §4.3 (strip directory information from suggested names)
//! and the Windows file naming rules (reserved names and characters).

use std::io;
use std::path::{Component, Path, PathBuf};

/// Name used when nothing usable is left after cleaning.
pub const FALLBACK_NAME: &str = "download";

/// Longest name we produce, in UTF-8 bytes. Common file systems allow 255;
/// we keep room for the `.zajil-part` and `.zajil-state` suffixes.
pub const MAX_NAME_BYTES: usize = 200;

/// Longest extension kept when a name is shortened, in UTF-8 bytes.
const MAX_KEPT_EXTENSION_BYTES: usize = 20;

/// Device names Windows reserves in any folder, with or without an extension.
const WINDOWS_RESERVED: &[&str] = &[
    "CON", "PRN", "AUX", "NUL", "CONIN$", "CONOUT$", "COM0", "COM1", "COM2", "COM3", "COM4",
    "COM5", "COM6", "COM7", "COM8", "COM9", "COM¹", "COM²", "COM³", "LPT0", "LPT1", "LPT2", "LPT3",
    "LPT4", "LPT5", "LPT6", "LPT7", "LPT8", "LPT9", "LPT¹", "LPT²", "LPT³",
];

/// Turns an untrusted name into a single safe file name. Never fails and
/// never returns an empty string.
///
/// - Keeps only the last path segment (after `/` or `\`).
/// - Removes control characters and invisible text-direction marks such as
///   U+202E, which can disguise `exe` as `txt`.
/// - Replaces characters Windows forbids (`<>:"|?*`) with `_`.
/// - Removes leading dots and spaces (no hidden files, no `..`) and trailing
///   dots and spaces (Windows drops them silently).
/// - Prefixes Windows reserved names (`CON`, `NUL.txt`, …) with `_`.
/// - Shortens the name to [`MAX_NAME_BYTES`], keeping a short extension.
pub fn sanitize_file_name(raw: &str) -> String {
    let last_segment = raw.rsplit(['/', '\\']).next().unwrap_or_default();

    // Joiners are judged after everything else is gone, against the
    // characters they would actually sit between.
    let cleaned: String = last_segment
        .chars()
        .filter(|&c| is_joiner(c) || !is_invisible_or_control(c))
        .map(|c| if is_forbidden(c) { '_' } else { c })
        .collect();

    let mut name = tidy(cleaned);
    if name.len() > MAX_NAME_BYTES {
        name = tidy(shorten(&name));
    }
    if name.is_empty() {
        return FALLBACK_NAME.to_owned();
    }
    if is_windows_reserved(&name) {
        name.insert(0, '_');
    }
    name
}

/// Errors from [`confine`].
#[derive(Debug, thiserror::Error)]
pub enum ConfineError {
    #[error("refusing to write outside the download folder: {name:?}")]
    OutsideFolder { name: String },
    #[error("refusing to write through a symbolic link: {}", path.display())]
    SymbolicLink { path: PathBuf },
    #[error("could not check the download folder: {0}")]
    Io(#[from] io::Error),
}

/// Joins `name` to the existing folder `dir` and checks that the result is a
/// direct child of `dir` that is not a symbolic link.
///
/// `name` should come from [`sanitize_file_name`]; this is the second line of
/// defence and rejects instead of repairing.
pub fn confine(dir: &Path, name: &str) -> Result<PathBuf, ConfineError> {
    let outside = || ConfineError::OutsideFolder {
        name: name.to_owned(),
    };
    let mut components = Path::new(name).components();
    match (components.next(), components.next()) {
        (Some(Component::Normal(part)), None) if part == name => {}
        _ => return Err(outside()),
    }
    if name.contains(['/', '\\']) {
        return Err(outside());
    }

    // `dunce` avoids Windows `\\?\` paths where an ordinary one is
    // equivalent, so the paths we report stay readable and usable.
    let dir = dunce::canonicalize(dir)?;
    let path = dir.join(name);
    match path.symlink_metadata() {
        Ok(meta) if meta.file_type().is_symlink() => {
            return Err(ConfineError::SymbolicLink { path });
        }
        Ok(_) => {}
        Err(err) if err.kind() == io::ErrorKind::NotFound => {}
        Err(err) => return Err(err.into()),
    }
    if path.parent() != Some(dir.as_path()) {
        return Err(outside());
    }
    Ok(path)
}

/// Trims the edges and drops misplaced joiners until neither changes the
/// name: removing a joiner can expose a trailing dot, and trimming can leave
/// a joiner at an edge.
fn tidy(mut name: String) -> String {
    loop {
        let next = trim_edges(&keep_allowed_joiners(trim_edges(&name))).to_owned();
        if next == name {
            return name;
        }
        name = next;
    }
}

const ZWNJ: char = '\u{200C}';
const ZWJ: char = '\u{200D}';

fn is_joiner(c: char) -> bool {
    c == ZWNJ || c == ZWJ
}

/// Keeps a ZWNJ or ZWJ only where it is part of normal writing, by the
/// ContextJ rules of RFC 5892 (Appendix A.1 and A.2), and never at an edge
/// of the name or next to a dot (where it would only hide something):
/// - ZWNJ or ZWJ right after a virama (Indic scripts);
/// - ZWNJ between a left- or dual-joining letter and a right- or
///   dual-joining letter, with only transparent marks in between (Arabic
///   script: `می‌خواهم`).
///
/// Every other joiner is removed (`invoice\u{200C}.pdf` → `invoice.pdf`).
/// Emoji sequences joined with ZWJ are not allowed by these rules and are
/// split into their parts.
fn keep_allowed_joiners(name: &str) -> String {
    let chars: Vec<char> = name.chars().collect();
    chars
        .iter()
        .enumerate()
        .filter(|&(i, &c)| !is_joiner(c) || joiner_allowed(&chars, i))
        .map(|(_, &c)| c)
        .collect()
}

fn joiner_allowed(chars: &[char], i: usize) -> bool {
    use icu_properties::CodePointMapData;
    use icu_properties::props::{CanonicalCombiningClass, JoiningType};

    let (Some(&before), Some(&after)) = (chars.get(i.wrapping_sub(1)), chars.get(i + 1)) else {
        return false; // first or last character of the name
    };
    if before == '.' || after == '.' {
        return false;
    }
    // A.1 and A.2: Canonical_Combining_Class(Before(cp)) = Virama.
    let combining = CodePointMapData::<CanonicalCombiningClass>::new();
    if combining.get(before) == CanonicalCombiningClass::Virama {
        return true;
    }
    if chars[i] != ZWNJ {
        return false; // A.2 allows ZWJ only after a virama
    }
    // A.1: (Joining_Type:{L,D})(Joining_Type:T)*\u200C(Joining_Type:T)*(Joining_Type:{R,D})
    let joining = CodePointMapData::<JoiningType>::new();
    let not_transparent = |t: &JoiningType| *t != JoiningType::Transparent;
    let left = chars[..i]
        .iter()
        .rev()
        .map(|&c| joining.get(c))
        .find(not_transparent);
    let right = chars[i + 1..]
        .iter()
        .map(|&c| joining.get(c))
        .find(not_transparent);
    matches!(
        left,
        Some(JoiningType::LeftJoining | JoiningType::DualJoining)
    ) && matches!(
        right,
        Some(JoiningType::RightJoining | JoiningType::DualJoining)
    )
}

/// Control characters (Cc) and every invisible format character (Cf), line
/// separator (Zl) and paragraph separator (Zp), as of Unicode 16. These let
/// two different names look the same (`invoice\u{200B}.pdf`), hide the real
/// extension (U+202E) or break a name across lines in a terminal (U+2028).
/// ZWNJ and ZWJ (U+200C, U+200D) are in the list too, but
/// [`keep_allowed_joiners`] keeps them where the script needs them.
fn is_invisible_or_control(c: char) -> bool {
    c.is_control() || INVISIBLE.iter().any(|range| range.contains(&c))
}

/// Unicode categories Cf, Zl and Zp.
const INVISIBLE: &[std::ops::RangeInclusive<char>] = &[
    '\u{00AD}'..='\u{00AD}',   // soft hyphen
    '\u{0600}'..='\u{0605}',   // Arabic number signs
    '\u{061C}'..='\u{061C}',   // Arabic letter mark
    '\u{06DD}'..='\u{06DD}',   // Arabic end of ayah
    '\u{070F}'..='\u{070F}',   // Syriac abbreviation mark
    '\u{0890}'..='\u{0891}',   // Arabic pound and piastre marks above
    '\u{08E2}'..='\u{08E2}',   // Arabic disputed end of ayah
    '\u{180E}'..='\u{180E}',   // Mongolian vowel separator
    '\u{200B}'..='\u{200F}',   // zero-width space, joiners, LRM, RLM
    '\u{2028}'..='\u{202E}',   // line/paragraph separators, embeddings, overrides
    '\u{2060}'..='\u{2064}',   // word joiner, invisible operators
    '\u{2066}'..='\u{206F}',   // isolates and deprecated format characters
    '\u{FEFF}'..='\u{FEFF}',   // byte order mark
    '\u{FFF9}'..='\u{FFFB}',   // interlinear annotation
    '\u{110BD}'..='\u{110BD}', // Kaithi number sign
    '\u{110CD}'..='\u{110CD}', // Kaithi number sign above
    '\u{13430}'..='\u{1343F}', // Egyptian hieroglyph format controls
    '\u{1BCA0}'..='\u{1BCA3}', // shorthand format controls
    '\u{1D173}'..='\u{1D17A}', // musical symbol format controls
    '\u{E0001}'..='\u{E0001}', // language tag
    '\u{E0020}'..='\u{E007F}', // tag characters
];

fn is_forbidden(c: char) -> bool {
    matches!(c, '<' | '>' | ':' | '"' | '|' | '?' | '*')
}

fn trim_edges(name: &str) -> &str {
    name.trim_start_matches(|c: char| c == '.' || c.is_whitespace())
        .trim_end_matches(|c: char| c == '.' || c.is_whitespace())
}

fn is_windows_reserved(name: &str) -> bool {
    // Windows ignores everything from the first dot, and trailing spaces
    // before it: `nul .txt` is also the NUL device.
    let stem = name.split('.').next().unwrap_or_default().trim_end();
    WINDOWS_RESERVED
        .iter()
        .any(|reserved| stem.eq_ignore_ascii_case(reserved))
}

/// Cuts `name` to [`MAX_NAME_BYTES`] at a character boundary, keeping the
/// extension when it is short.
fn shorten(name: &str) -> String {
    let extension = name
        .rfind('.')
        .map(|dot| &name[dot..])
        .filter(|ext| ext.len() <= MAX_KEPT_EXTENSION_BYTES && ext.len() > 1)
        .unwrap_or("");
    let stem = &name[..name.len() - extension.len()];
    let budget = MAX_NAME_BYTES - extension.len();
    let mut cut = budget.min(stem.len());
    while !stem.is_char_boundary(cut) {
        cut -= 1;
    }
    format!("{}{}", &stem[..cut], extension)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keeps_ordinary_names() {
        for name in [
            "report.pdf",
            "تقرير الشهر.pdf",
            "Setup 2.0 (x64).zip",
            "a b",
        ] {
            assert_eq!(sanitize_file_name(name), name);
        }
    }

    #[test]
    fn removes_every_invisible_format_and_separator_character() {
        // Review of #12: these passed through before (U+200B, U+2060,
        // U+00AD, U+2028, U+2029, U+180E, U+FFF9..U+FFFB, tags …).
        let mut checked = 0;
        for range in INVISIBLE {
            for c in range.clone() {
                let raw = format!("in{c}voice.pdf");
                assert_eq!(
                    sanitize_file_name(&raw),
                    "invoice.pdf",
                    "U+{:04X}",
                    c as u32
                );
                checked += 1;
            }
        }
        assert!(checked >= 170, "{checked}");
        // The ones named in the review, explicitly.
        for c in [
            '\u{200B}', '\u{2028}', '\u{2029}', '\u{FEFF}', '\u{2060}', '\u{00AD}',
        ] {
            assert_eq!(
                sanitize_file_name(&format!("a{c}b")),
                "ab",
                "U+{:04X}",
                c as u32
            );
        }
    }

    #[test]
    fn invisible_characters_cannot_make_two_names_look_alike() {
        assert_eq!(
            sanitize_file_name("invoice\u{200B}.pdf"),
            sanitize_file_name("invoice.pdf")
        );
        assert_eq!(sanitize_file_name("invoice\u{200C}.pdf"), "invoice.pdf");
    }

    #[test]
    fn keeps_joiners_where_rfc_5892_allows_them() {
        for name in [
            // Persian: ZWNJ between dual-joining yeh and khah.
            "می\u{200C}خواهم",
            "می\u{200C}خواهم.txt",
            // With a transparent mark (fatha) before the ZWNJ.
            "بَ\u{200C}ب",
            // Devanagari: ZWNJ and ZWJ right after a virama.
            "क्\u{200C}ष",
            "क्\u{200D}ष.pdf",
        ] {
            assert_eq!(sanitize_file_name(name), name, "{name:?}");
        }
    }

    #[test]
    fn removes_joiners_everywhere_else() {
        for (raw, safe) in [
            // Next to the dot of the extension.
            ("invoice\u{200C}.pdf", "invoice.pdf"),
            ("ملف\u{200C}.zip", "ملف.zip"),
            ("ملف.\u{200C}zip", "ملف.zip"),
            ("क्\u{200D}.pdf", "क्.pdf"),
            // At the start or the end of the name.
            ("\u{200C}می", "می"),
            ("می\u{200C}", "می"),
            ("क्\u{200D}", "क्"),
            ("\u{200D}a", "a"),
            // Between letters that do not join.
            ("a\u{200C}b", "ab"),
            ("a\u{200D}b", "ab"),
            ("د\u{200C}ا", "دا"), // dal and alef join on one side only
            ("ب\u{200D}ب", "بب"), // ZWJ is only allowed after a virama
            ("ب\u{200C}\u{200C}ب", "بب"),
            // Emoji sequences are not allowed by RFC 5892.
            ("👨\u{200D}👩", "👨👩"),
            // Removing a joiner must not leave a trailing dot.
            ("a.\u{200C}", "a"),
        ] {
            assert_eq!(sanitize_file_name(raw), safe, "{raw:?}");
        }
    }

    #[test]
    fn strips_directories() {
        assert_eq!(sanitize_file_name("../../x"), "x");
        assert_eq!(sanitize_file_name("..\\..\\x"), "x");
        assert_eq!(sanitize_file_name("/etc/passwd"), "passwd");
        assert_eq!(sanitize_file_name("C:\\Windows\\win.ini"), "win.ini");
        assert_eq!(sanitize_file_name("\\\\server\\share\\f.txt"), "f.txt");
        assert_eq!(sanitize_file_name("dir/"), FALLBACK_NAME);
    }

    #[test]
    fn drive_letters_cannot_survive() {
        assert_eq!(sanitize_file_name("C:evil.txt"), "C_evil.txt");
    }

    #[test]
    fn rejects_dot_names() {
        for name in ["", ".", "..", "...", " . ", "../..", "  "] {
            assert_eq!(sanitize_file_name(name), FALLBACK_NAME, "{name:?}");
        }
        assert_eq!(sanitize_file_name(".bashrc"), "bashrc");
    }

    #[test]
    fn removes_text_direction_overrides() {
        assert_eq!(sanitize_file_name("a\u{202E}txt.exe"), "atxt.exe");
        assert_eq!(sanitize_file_name("\u{2067}x\u{2069}.exe"), "x.exe");
        assert_eq!(sanitize_file_name("\u{200F}\u{061C}x.exe"), "x.exe");
    }

    #[test]
    fn removes_control_characters() {
        assert_eq!(sanitize_file_name("a\0b\r\nc\u{7f}\u{85}.txt"), "abc.txt");
    }

    #[test]
    fn replaces_windows_forbidden_characters() {
        assert_eq!(sanitize_file_name("a<b>c:d\"e|f?g*h"), "a_b_c_d_e_f_g_h");
    }

    #[test]
    fn trims_trailing_dots_and_spaces() {
        assert_eq!(sanitize_file_name("file.txt. . "), "file.txt");
    }

    #[test]
    fn prefixes_windows_reserved_names() {
        for (raw, safe) in [
            ("CON", "_CON"),
            ("con", "_con"),
            ("NUL.txt", "_NUL.txt"),
            ("nul .txt", "_nul .txt"),
            ("COM1.tar.gz", "_COM1.tar.gz"),
            ("LPT9", "_LPT9"),
            ("com¹", "_com¹"),
            ("CONOUT$", "_CONOUT$"),
        ] {
            assert_eq!(sanitize_file_name(raw), safe, "{raw:?}");
        }
        for not_reserved in ["CONSOLE.txt", "nul-device", "COM10", "LPT"] {
            assert_eq!(sanitize_file_name(not_reserved), not_reserved);
        }
    }

    #[test]
    fn shortens_long_names_and_keeps_the_extension() {
        let long = format!("{}.zip", "a".repeat(1000));
        let safe = sanitize_file_name(&long);
        assert_eq!(safe.len(), MAX_NAME_BYTES);
        assert!(safe.ends_with(".zip"));
    }

    #[test]
    fn shortens_multibyte_names_on_a_character_boundary() {
        let long = format!("{}.pdf", "ملف".repeat(200));
        let safe = sanitize_file_name(&long);
        assert!(safe.len() <= MAX_NAME_BYTES);
        assert!(safe.ends_with(".pdf"));
        assert!(safe.starts_with("ملف"));
    }

    #[test]
    fn shortening_drops_a_long_extension() {
        let long = format!("x.{}", "e".repeat(500));
        let safe = sanitize_file_name(&long);
        assert_eq!(safe.len(), MAX_NAME_BYTES);
    }

    #[test]
    fn output_is_always_a_single_safe_name() {
        let nasty = [
            "../../x",
            "/abs/path",
            "C:\\abs\\path",
            "CON",
            "NUL.txt",
            "a\u{202E}txt.exe",
            &"z".repeat(5000),
            "..",
            "\u{0}",
            "a/../../b",
            "a\u{200B}\u{2028}\u{2029}\u{FEFF}\u{E0041}b",
        ];
        for raw in nasty {
            let safe = sanitize_file_name(raw);
            assert!(!safe.is_empty());
            assert!(safe.len() <= MAX_NAME_BYTES);
            assert!(!safe.contains(['/', '\\', ':', '\u{202E}', '\0']));
            assert!(
                !safe
                    .chars()
                    .any(|c| is_invisible_or_control(c) && !is_joiner(c)),
                "{safe:?}"
            );
            assert_eq!(keep_allowed_joiners(&safe), safe);
            assert!(!safe.starts_with('.'));
            assert!(!is_windows_reserved(&safe));
            let mut parts = Path::new(&safe).components();
            assert!(matches!(parts.next(), Some(Component::Normal(_))));
            assert!(parts.next().is_none());
        }
    }
}
