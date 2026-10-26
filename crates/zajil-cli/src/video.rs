//! `zajil video`: video downloads with yt-dlp and Zajil's ffmpeg
//! (ADR-0009). The tools are downloaded only after the user agrees
//! (`zajil video tools install`), and checked before every run.

use std::io::{BufRead, IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use clap::Subcommand;
use zajil_core::tools::{Tool, ToolError, Toolbox};
use zajil_core::video::{Choice, Download, Probed, Runs, Video, VideoEvent, VideoRunner};
use zajil_core::{Engine, Stop};

#[derive(Debug, Subcommand)]
pub enum VideoCommand {
    /// List the formats (qualities) a page offers.
    Formats {
        /// The page or video URL, http:// or https://.
        url: String,
    },
    /// Download a video. Without a choice, the best picture with sound.
    Get {
        /// The page or video URL, http:// or https://.
        url: String,
        /// A format id from `zajil video formats`.
        #[arg(long, value_name = "ID", conflicts_with_all = ["video", "audio_only"])]
        format: Option<String>,
        /// A picture-only format id, merged with --audio without re-encoding.
        #[arg(
            long,
            value_name = "ID",
            requires = "audio",
            conflicts_with = "audio_only"
        )]
        video: Option<String>,
        /// A sound-only format id, with --video.
        #[arg(long, value_name = "ID", requires = "video")]
        audio: Option<String>,
        /// Sound only, as m4a (AAC).
        #[arg(long)]
        audio_only: bool,
        /// Folder to save into. Defaults to the system Downloads folder.
        #[arg(short, long, value_name = "FOLDER")]
        dir: Option<PathBuf>,
    },
    /// Show, download or remove the video tools.
    Tools {
        #[command(subcommand)]
        command: Option<ToolsCommand>,
    },
}

#[derive(Debug, Subcommand)]
pub enum ToolsCommand {
    /// Download yt-dlp and ffmpeg (asks first, unless --yes).
    Install {
        /// Agree without being asked.
        #[arg(long)]
        yes: bool,
    },
    /// Remove every downloaded tool.
    Remove,
}

/// What each tool is, for the consent text: its license and its source.
fn about(tool: Tool) -> &'static str {
    match tool {
        Tool::YtDlp => "yt-dlp, GPLv3+ as built (Unlicense source), from github.com/yt-dlp/yt-dlp",
        Tool::Ffmpeg | Tool::Ffprobe => {
            "FFmpeg (ffmpeg and ffprobe), LGPL 2.1 or later, Zajil's build with its source, from github.com/motalim-ai/zajil-download-manager"
        }
        Tool::QuickJs => {
            "QuickJS-NG (qjs), MIT, for YouTube's JavaScript when neither Deno nor Node is installed, run in a sandbox, from github.com/quickjs-ng/quickjs"
        }
    }
}

/// Says so when QuickJS-NG ran without all of its sandbox on this system
/// (owner's condition: it runs, and that is told).
fn tell_sandbox(runner: &VideoRunner) {
    if let Some(held) = runner.js_sandbox()
        && !(held.programs && held.files && held.network)
    {
        let yes = |b: bool| if b { "yes" } else { "no" };
        eprintln!(
            "note: on this system QuickJS-NG ran with only part of its sandbox (other programs refused: {}; writing elsewhere refused: {}; network refused: {})",
            yes(held.programs),
            yes(held.files),
            yes(held.network),
        );
    }
}

pub async fn run(command: VideoCommand, data_dir: &Path, engine: Engine) -> ExitCode {
    let toolbox = Toolbox::new(data_dir.join("tools"));
    let runs = Runs::new(data_dir.join("run"));
    // What a crash left: process trees ended, folders removed.
    let cleaned = {
        let (tools_root, runs_root) = (data_dir.join("tools"), data_dir.join("run"));
        tokio::task::spawn_blocking(move || {
            Toolbox::new(tools_root).remove_leftovers()?;
            Runs::new(runs_root).clean_leftovers()
        })
        .await
    };
    if let Ok(Err(err)) | Err(err) = cleaned.map_err(std::io::Error::other) {
        return crate::fail(&format!("could not clean up an earlier run: {err}"));
    }
    match command {
        VideoCommand::Tools { command: None } => status(&toolbox),
        VideoCommand::Tools {
            command: Some(ToolsCommand::Install { yes }),
        } => install(&toolbox, &engine, yes).await,
        VideoCommand::Tools {
            command: Some(ToolsCommand::Remove),
        } => match toolbox.remove_all() {
            Ok(()) => {
                println!("Removed the video tools.");
                ExitCode::SUCCESS
            }
            Err(err) => crate::fail(&format!("could not remove the tools: {err}")),
        },
        VideoCommand::Formats { url } => {
            let runner = match runner(&data_dir.join("tools"), runs, engine).await {
                Ok(runner) => runner,
                Err(code) => return code,
            };
            let probed = runner.probe(&url, None, &stop_on_ctrl_c()).await;
            tell_sandbox(&runner);
            match probed {
                Ok(Probed::Video(video)) => {
                    print_formats(&video);
                    ExitCode::SUCCESS
                }
                Ok(Probed::Playlist { title, entries }) => {
                    println!(
                        "{} (a playlist of {} items):",
                        one_line(&title),
                        entries.len()
                    );
                    for entry in entries {
                        let url = entry.url.as_deref().map(item_link).unwrap_or_default();
                        println!(
                            "  {}  {url}",
                            one_line(entry.title.as_deref().unwrap_or(""))
                        );
                    }
                    ExitCode::SUCCESS
                }
                Err(err) => crate::fail(&err.to_string()),
            }
        }
        VideoCommand::Get {
            url,
            format,
            video,
            audio,
            audio_only,
            dir,
        } => {
            let runner = match runner(&data_dir.join("tools"), runs, engine).await {
                Ok(runner) => runner,
                Err(code) => return code,
            };
            let stop = stop_on_ctrl_c();
            let probed = runner.probe(&url, None, &stop).await;
            tell_sandbox(&runner);
            let found = match probed {
                Ok(Probed::Video(video)) => video,
                Ok(Probed::Playlist { entries, .. }) => {
                    return crate::fail(&format!(
                        "this is a playlist of {} items: download them one by one (`zajil video formats` lists them)",
                        entries.len()
                    ));
                }
                Err(err) => return crate::fail(&err.to_string()),
            };
            let choice = match (format, video, audio, audio_only) {
                (Some(id), ..) => Choice::Format(id),
                (_, Some(video), Some(audio), _) => Choice::Merge { video, audio },
                (.., true) => Choice::AudioOnly,
                _ => match found.best_choice() {
                    Some(choice) => choice,
                    None => return crate::fail("the page offers no format Zajil can download"),
                },
            };
            let dest = match crate::download_dir(dir.as_deref()) {
                Ok(dir) => dir,
                Err(message) => return crate::fail(&message),
            };
            let request = Download {
                url: &url,
                choice: &choice,
                formats: &found.formats,
                cookies: None,
                dest: &dest,
                name: &found.title,
            };
            let mut progress = Progress::new();
            let downloaded = runner
                .download(&request, &stop, &mut |event| progress.show(event))
                .await;
            tell_sandbox(&runner);
            match downloaded {
                Ok(file) => {
                    progress.end();
                    println!("Saved {}", file.display());
                    ExitCode::SUCCESS
                }
                Err(err) => {
                    progress.end();
                    crate::fail(&err.to_string())
                }
            }
        }
    }
}

/// A runner for the tools folder, or a message on how to get the
/// tools. The runner checks the tools again before every run; this
/// first look is only for a clear message when they are missing.
async fn runner(tools_root: &Path, runs: Runs, engine: Engine) -> Result<VideoRunner, ExitCode> {
    let toolbox = Toolbox::new(tools_root);
    let checked = async {
        for tool in [Tool::YtDlp, Tool::Ffmpeg, Tool::Ffprobe] {
            toolbox.verified(tool).await?;
        }
        // QuickJS-NG only for YouTube: tools installed before it came still
        // work without it.
        let has_qjs = match toolbox.verified(Tool::QuickJs).await {
            Ok(_) => true,
            Err(ToolError::Missing(_)) => false,
            Err(err) => return Err(err),
        };
        let js = tokio::task::spawn_blocking(move || {
            let jsbox = std::env::current_exe()
                .ok()
                .and_then(|exe| zajil_core::video::jsbox_beside(&exe))
                .filter(|_| has_qjs);
            let path = std::env::var_os("PATH");
            // The CLI has no settings: «استعمل QuickJS-NG دائمًا» is the app's.
            zajil_core::video::find_js_runtime(path.as_deref(), jsbox, false)
        })
        .await
        .map_err(std::io::Error::other)?;
        Ok::<_, ToolError>(js)
    };
    match checked.await {
        Ok(js) => Ok(VideoRunner::new(toolbox, runs, engine).with_js(js)),
        Err(ToolError::Missing(_)) => Err(crate::fail(
            "the video tools are not downloaded yet: run `zajil video tools install`",
        )),
        Err(err) => Err(crate::fail(&err.to_string())),
    }
}

fn status(toolbox: &Toolbox) -> ExitCode {
    for tool in Tool::ALL {
        let state = match toolbox.path(tool) {
            Ok(path) if toolbox.is_installed(tool) => format!("ready, {}", path.display()),
            Ok(_) => "not downloaded".to_owned(),
            Err(err) => err.to_string(),
        };
        println!("{tool}: {state}");
    }
    ExitCode::SUCCESS
}

async fn install(toolbox: &Toolbox, engine: &Engine, yes: bool) -> ExitCode {
    let missing: Vec<Tool> = Tool::ALL
        .into_iter()
        .filter(|t| !toolbox.is_installed(*t))
        .collect();
    if missing.is_empty() {
        println!("The video tools are ready.");
        return ExitCode::SUCCESS;
    }
    println!("Zajil needs these programs to download videos:");
    let mut described = Vec::new();
    for tool in &missing {
        if !described.contains(&about(*tool)) {
            println!("  - {}", about(*tool));
            described.push(about(*tool));
        }
    }
    println!(
        "Each is checked against the SHA-256 written in Zajil, before it is kept and before every run."
    );
    if !yes {
        if !std::io::stdin().is_terminal() {
            return crate::fail("not asked on a terminal: add --yes to agree");
        }
        print!("Download them now? [y/N] ");
        let _ = std::io::stdout().flush();
        let mut answer = String::new();
        if std::io::stdin().lock().read_line(&mut answer).is_err()
            || !matches!(answer.trim(), "y" | "Y" | "yes")
        {
            println!("Nothing was downloaded.");
            return ExitCode::from(1);
        }
    }
    // Every tool needs a released file for this system before anything is
    // downloaded: yt-dlp alone (about 35 MB) is of no use without FFmpeg
    // (review of #128). Every tool is pinned for every target this build
    // knows; on another system, nothing comes.
    let unavailable: Vec<String> = Tool::ALL
        .into_iter()
        .filter_map(|tool| toolbox.path(tool).err().map(|err| format!("{tool}: {err}")))
        .collect();
    if !unavailable.is_empty() {
        return crate::fail(&format!(
            "video downloads are not available on this system ({}); nothing was downloaded",
            unavailable.join("; ")
        ));
    }
    let stop = stop_on_ctrl_c();
    for tool in missing {
        let mut progress = Progress::new();
        match toolbox
            .install(engine, tool, &stop, &mut |event| {
                if let zajil_core::Event::Progress { done, total } = event {
                    progress.show(VideoEvent::Progress {
                        done,
                        total,
                        speed: None,
                    });
                }
            })
            .await
        {
            Ok(path) => {
                progress.end();
                println!("{tool}: ready, {}", path.display());
            }
            Err(err) => {
                progress.end();
                return crate::fail(&format!("{tool}: {err}"));
            }
        }
    }
    ExitCode::SUCCESS
}

fn print_formats(video: &Video) {
    println!("{}", one_line(&video.title));
    println!(
        "{:<20} {:<5} {:<10} {:<6} {:<14} {:>10}",
        "ID", "EXT", "PICTURE", "FPS", "SOUND", "SIZE"
    );
    for f in &video.formats {
        let picture = match (f.has_video(), f.width, f.height) {
            (false, ..) if f.vcodec.is_some() => "none".to_owned(),
            (_, Some(w), Some(h)) => format!("{w}x{h}"),
            (_, None, Some(h)) => format!("{h}p"),
            _ => "?".to_owned(),
        };
        let sound = if f.has_audio() {
            f.acodec.clone().unwrap_or_default()
        } else if f.acodec.is_some() {
            "none".to_owned()
        } else {
            "?".to_owned()
        };
        let size = f
            .filesize
            .map(crate::human_bytes)
            .or_else(|| {
                f.filesize_approx
                    .map(|b| format!("~{}", crate::human_bytes(b)))
            })
            .unwrap_or_default();
        let fps = f.fps.map(|v| format!("{v:.0}")).unwrap_or_default();
        println!(
            "{:<20} {:<5} {:<10} {:<6} {:<14} {:>10}",
            one_line(&f.id),
            one_line(&f.ext),
            picture,
            fps,
            one_line(&sound),
            size
        );
    }
}

/// Text from a page, safe to print on one terminal line: no control
/// characters (they drive the terminal), no invisible direction marks
/// (they reorder what is shown).
/// A playlist item's link as `zajil video formats` lists it, to download
/// it with `zajil video get` (#150: printed without its query, a YouTube
/// item was `https://www.youtube.com/watch`, with nothing to download).
/// Its query is kept, unlike the links Zajil prints elsewhere: these are
/// the playlist's own page links, listed so the user can use them. A user
/// name or password is never printed; a link that is not http or https is
/// printed as the others are.
fn item_link(url: &str) -> String {
    match zajil_core::Url::parse(url) {
        Ok(mut link) if matches!(link.scheme(), "http" | "https") => {
            let _ = link.set_username("");
            let _ = link.set_password(None);
            one_line(link.as_str())
        }
        _ => zajil_core::queue::shown_url(url),
    }
}

fn one_line(text: &str) -> String {
    text.chars()
        .filter(|c| {
            !c.is_control()
                && !matches!(c, '\u{061C}' | '\u{200E}' | '\u{200F}' | '\u{202A}'..='\u{202E}' | '\u{2066}'..='\u{2069}')
        })
        .collect()
}

/// A stop requested by Ctrl+C.
fn stop_on_ctrl_c() -> Stop {
    let stop = Stop::new();
    let on_signal = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_signal.request();
        }
    });
    stop
}

/// One progress line, rewritten in place on a terminal.
struct Progress {
    terminal: bool,
    shown: bool,
}

impl Progress {
    fn new() -> Self {
        Progress {
            terminal: std::io::stderr().is_terminal(),
            shown: false,
        }
    }

    fn show(&mut self, event: VideoEvent) {
        if !self.terminal {
            return;
        }
        let VideoEvent::Progress { done, total, speed } = event;
        let mut line = crate::human_bytes(done);
        if let Some(total) = total {
            line.push_str(&format!(" of {}", crate::human_bytes(total)));
        }
        if let Some(speed) = speed {
            line.push_str(&format!(", {}/s", crate::human_bytes(speed)));
        }
        eprint!("\r\x1b[2K{line}");
        self.shown = true;
    }

    fn end(&mut self) {
        if self.shown {
            eprintln!();
            self.shown = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// #150: a playlist item's link keeps what makes it the item, never a
    /// user name or password.
    #[test]
    fn a_playlist_item_is_listed_with_its_whole_link() {
        assert_eq!(
            item_link("https://www.youtube.com/watch?v=dQw4w9WgXcQ"),
            "https://www.youtube.com/watch?v=dQw4w9WgXcQ"
        );
        assert_eq!(
            item_link("https://me:secret@videos.example/v/7?list=x"),
            "https://videos.example/v/7?list=x"
        );
        assert!(!item_link("https://me:secret@videos.example/v/7").contains("secret"));
    }
}
