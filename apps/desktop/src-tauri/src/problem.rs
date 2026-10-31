//! «Copy problem info» (owner's decision, 2026-10-31: copied, never sent
//! by Zajil): what a report needs, on the clipboard for the user to paste
//! wherever they ask for help. Zajil's version, the system, the language,
//! the video tools, an extension that said it is old, and the log's last
//! lines. The log never holds a link or a cookie; a link that got in
//! anyway, and the user's home folder (their name), are taken out here.

use std::io::{Read, Seek, SeekFrom};
use std::path::Path;

use tauri::{Manager, State};
use zajil_platform::{AppPaths, SystemPaths};

use crate::App;

/// How many of the log's last lines go.
const LOG_LINES: usize = 100;
/// The most of the log's end read for them.
const LOG_TAIL: u64 = 64 * 1024;

/// What the text says of the program and the system.
pub struct Facts<'a> {
    pub version: &'a str,
    pub os: &'a str,
    pub arch: &'a str,
    pub language: &'a str,
    /// Each video tool: its name, its version, and whether it is there.
    pub tools: Vec<(String, Option<String>, bool)>,
    /// The extension's version, when it said it is too old.
    pub extension_old: Option<String>,
}

/// The text to copy: `facts`, then the last lines of `log`, with no link
/// and no home folder.
pub fn text(facts: &Facts<'_>, log: &str, home: Option<&Path>) -> String {
    let mut out = vec![
        format!("Zajil {}", facts.version),
        format!("System: {} {}", facts.os, facts.arch),
        format!("Language: {}", facts.language),
    ];
    let tools: Vec<String> = facts
        .tools
        .iter()
        .map(|(name, version, installed)| {
            format!(
                "{name} {}{}",
                version.as_deref().unwrap_or("?"),
                if *installed { "" } else { " (not installed)" }
            )
        })
        .collect();
    out.push(format!("Video tools: {}", tools.join(", ")));
    if let Some(old) = &facts.extension_old {
        out.push(format!(
            "Browser extension: {old} (older than this Zajil needs)"
        ));
    }
    out.push(String::new());
    out.push(format!("The log's last {LOG_LINES} lines:"));
    let lines: Vec<&str> = log.lines().collect();
    let from = lines.len().saturating_sub(LOG_LINES);
    for line in &lines[from..] {
        out.push(clean(line, home));
    }
    out.join("\n")
}

/// `line` with each link replaced by `(link)`, and the home folder by `~`.
fn clean(line: &str, home: Option<&Path>) -> String {
    let words: Vec<String> = line
        .split(' ')
        .map(|w| {
            let lower = w.to_ascii_lowercase();
            if ["http://", "https://", "ftp://", "file://"]
                .iter()
                .any(|s| lower.contains(s))
            {
                "(link)".to_owned()
            } else {
                w.to_owned()
            }
        })
        .collect();
    let mut line = words.join(" ");
    if let Some(home) = home.and_then(|h| h.to_str()).filter(|h| h.len() > 1) {
        line = line.replace(home, "~");
    }
    line
}

/// The log's last part, up to [`LOG_TAIL`] bytes, from a whole line.
fn log_tail(file: &Path) -> String {
    let Ok(mut f) = zajil_platform::fs::open_regular_nofollow(file, false) else {
        return String::new();
    };
    let len = f.metadata().map(|m| m.len()).unwrap_or(0);
    let start = len.saturating_sub(LOG_TAIL);
    if f.seek(SeekFrom::Start(start)).is_err() {
        return String::new();
    }
    let mut bytes = Vec::new();
    let _ = f.take(LOG_TAIL).read_to_end(&mut bytes);
    let text = String::from_utf8_lossy(&bytes).into_owned();
    if start > 0 {
        // Cut in a line: from the next one.
        text.split_once('\n')
            .map(|(_, rest)| rest.to_owned())
            .unwrap_or_default()
    } else {
        text
    }
}

/// «Copy problem info»: the text on the clipboard.
#[tauri::command]
pub async fn copy_problem_info<R: tauri::Runtime>(
    app: State<'_, App>,
    handle: tauri::AppHandle<R>,
) -> Result<(), String> {
    let version = handle.package_info().version.to_string();
    let settings = app.settings.clone();
    let language = tokio::task::spawn_blocking(move || settings.load().map(|s| s.language))
        .await
        .ok()
        .and_then(Result::ok)
        .map(|l| format!("{l:?}").to_ascii_lowercase())
        .unwrap_or_else(|| "?".into());
    let tools = crate::video::video_tools_status(handle.state::<App>())
        .await
        .unwrap_or_default()
        .into_iter()
        .map(|t| (t.tool, t.version, t.installed))
        .collect();
    let extension_old = handle.state::<crate::update::ExtensionOld>().version();
    let (log, home) = tokio::task::spawn_blocking(|| {
        let log = SystemPaths::new()
            .map(|p| log_tail(&p.log_dir().join("zajil.log")))
            .unwrap_or_default();
        (log, std::env::home_dir())
    })
    .await
    .map_err(|e| e.to_string())?;
    let facts = Facts {
        version: &version,
        os: std::env::consts::OS,
        arch: std::env::consts::ARCH,
        language: &language,
        tools,
        extension_old,
    };
    crate::copy_text(&app, text(&facts, &log, home.as_deref())).await
}

#[cfg(test)]
mod tests {
    use super::*;

    /// What is copied says the program, the system and the tools, keeps
    /// the log's last lines only, and never a link or the home folder.
    #[test]
    fn the_text_has_the_facts_and_no_link_nor_home() {
        let facts = Facts {
            version: "1.2.1",
            os: "macos",
            arch: "aarch64",
            language: "ar",
            tools: vec![
                ("yt-dlp".into(), Some("2026.10.21".into()), true),
                ("ffmpeg".into(), Some("8.1.3".into()), false),
            ],
            extension_old: Some("1.2.1".into()),
        };
        let mut log: Vec<String> = (0..150).map(|i| format!("zajil: line {i}")).collect();
        log.push("zajil: could not read https://example.com/v?token=SECRET now".into());
        log.push("zajil: saved /Users/hussein/Downloads/x.mp4".into());
        let home = Path::new("/Users/hussein");
        let got = text(&facts, &log.join("\n"), Some(home));
        assert!(got.starts_with("Zajil 1.2.1\nSystem: macos aarch64\nLanguage: ar\n"));
        assert!(got.contains("Video tools: yt-dlp 2026.10.21, ffmpeg 8.1.3 (not installed)"));
        assert!(got.contains("Browser extension: 1.2.1 (older than this Zajil needs)"));
        assert!(!got.contains("line 51\n"), "only the last lines");
        assert!(got.contains("line 149"));
        assert!(got.contains("zajil: could not read (link) now"));
        assert!(!got.contains("SECRET") && !got.contains("example.com"));
        assert!(got.contains("zajil: saved ~/Downloads/x.mp4"));
        assert!(!got.contains("hussein"));
    }
}
