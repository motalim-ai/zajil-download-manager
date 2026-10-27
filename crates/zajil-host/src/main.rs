//! `zajil-host`: the browser starts it for Zajil's extension (ADR-0005).
//! It reads the extension's messages from stdin, checks each one
//! (`zajil_host::parse`), hands a download to the running app over the
//! user's private channel, and writes every answer to stdout. It never
//! downloads anything itself.
//!
//! The data folder is the app's (`ZAJIL_DATA_DIR`, else the user's
//! standard one), so the host finds the app that the user runs. When no
//! app is running, the host starts the one next to it, unless
//! `ZAJIL_NO_LAUNCH` is set (tests), and waits for it to open its channel.
//!
//! A `link` message (M5, `runtime.connectNative`) turns the host into the
//! extension's link to the app for the rest of its life ([`link`]): Zajil
//! asks that browser for cookies on it. The link never starts the app.
//!
//! Nothing is written to stderr that the browser would need: Chrome keeps
//! it in its log, Firefox in the browser console.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use zajil_host::{
    CHANNEL_TIMEOUT, FrameError, FromLink, LINK_ANSWER, MAX_REQUEST, MAX_RESPONSE,
    PROTOCOL_VERSION, Request, Response, ToExtension, parse, read_frame, write_frame, write_json,
};
use zajil_platform::AppPaths;

/// How long to wait for an app that is being started.
const START_PATIENCE: Duration = Duration::from_secs(20);
/// How often the link looks for the app while it is not running.
const LINK_RETRY: Duration = Duration::from_secs(2);

fn main() -> ExitCode {
    let data_dir = match data_dir() {
        Ok(dir) => dir,
        Err(err) => {
            eprintln!("zajil-host: no data folder: {err}");
            return ExitCode::FAILURE;
        }
    };
    let (mut input, mut output) = (io::stdin().lock(), io::stdout().lock());
    loop {
        let message = match read_frame(&mut input, MAX_REQUEST) {
            Ok(Some(message)) => message,
            // The browser closed the connection.
            Ok(None) => return ExitCode::SUCCESS,
            // Nothing after an oversized or broken message can be trusted
            // to start where a message starts: say why, then stop.
            Err(err @ FrameError::TooLarge(..)) => {
                let _ = write_json(&mut output, &Response::refused(err.to_string()));
                return ExitCode::FAILURE;
            }
            Err(FrameError::Io(err)) => {
                eprintln!("zajil-host: {err}");
                return ExitCode::FAILURE;
            }
        };
        let answer = match parse(&message) {
            Err(why) => write_json(&mut output, &Response::refused(why)),
            Ok(Request::Ping { .. }) => {
                write_json(&mut output, &Response::pong(language(&data_dir)))
            }
            // Only another start of Zajil asks for its window (#60).
            Ok(Request::ShowWindow { .. }) => {
                write_json(&mut output, &Response::refused("not from the browser"))
            }
            Ok(Request::Link { .. }) => {
                drop(input);
                return link(&data_dir, &message, output);
            }
            Ok(Request::Cookies { .. }) => {
                write_json(&mut output, &Response::refused("not on a link"))
            }
            Ok(request) => match forward(&data_dir, &request) {
                Ok(app_answer) => write_frame(&mut output, &app_answer),
                Err(err) => write_json(&mut output, &Response::refused(err)),
            },
        };
        if let Err(err) = answer {
            eprintln!("zajil-host: cannot answer the browser: {err}");
            return ExitCode::FAILURE;
        }
    }
}

/// The extension's link to the app (M5): connects to the app (waiting
/// for it while it is not running, again whenever it goes away), sends it
/// `hello` (the `link` message), then passes each of its questions to the
/// extension and the extension's answer back, one at a time; a question
/// the extension does not answer within LINK_ANSWER is answered
/// `no_answer`. Ends with the browser's connection.
fn link(data_dir: &Path, hello: &[u8], mut output: io::StdoutLock<'static>) -> ExitCode {
    // The extension's answers, read on their own thread.
    let (answers, answered) = std::sync::mpsc::channel::<(u64, Option<String>)>();
    std::thread::spawn(move || {
        let mut input = io::stdin().lock();
        loop {
            match read_frame(&mut input, MAX_REQUEST) {
                Ok(Some(message)) => match parse(&message) {
                    Ok(Request::Cookies { ask, cookies, .. }) => {
                        let _ = answers.send((ask, cookies));
                    }
                    Ok(_) => eprintln!("zajil-host: only answers go on the link"),
                    Err(why) => eprintln!("zajil-host: {why}"),
                },
                // The browser closed the link, or broke it: the host ends.
                Ok(None) => std::process::exit(0),
                Err(_) => std::process::exit(1),
            }
        }
    });
    loop {
        let mut app = match zajil_platform::ipc::connect(data_dir, Duration::ZERO) {
            Ok(app) => app,
            Err(_) => {
                std::thread::sleep(LINK_RETRY);
                continue;
            }
        };
        if write_frame(&mut app, hello).is_err() {
            std::thread::sleep(LINK_RETRY);
            continue;
        }
        while let Ok(Some(question)) = read_frame(&mut app, MAX_REQUEST) {
            let ask = match ToExtension::parse(&question) {
                Ok(question) => question.ask(),
                Err(why) => {
                    eprintln!("zajil-host: {why}");
                    break;
                }
            };
            // Answers to earlier questions that came too late are dropped.
            while answered.try_recv().is_ok() {}
            if write_frame(&mut output, &question).is_err() {
                return ExitCode::FAILURE;
            }
            let deadline = std::time::Instant::now() + LINK_ANSWER;
            let answer = loop {
                let left = deadline.saturating_duration_since(std::time::Instant::now());
                match answered.recv_timeout(left) {
                    Ok((got, cookies)) if got == ask => {
                        break FromLink::Cookies {
                            protocol_version: PROTOCOL_VERSION,
                            ask,
                            cookies,
                        };
                    }
                    Ok(_) => continue,
                    Err(_) => {
                        break FromLink::NoAnswer {
                            protocol_version: PROTOCOL_VERSION,
                            ask,
                        };
                    }
                }
            };
            if write_json(&mut app, &answer).is_err() {
                break;
            }
        }
        std::thread::sleep(LINK_RETRY);
    }
}

/// Zajil's language in its settings (`settings.json` in the data folder;
/// none yet is the default, Arabic), or none when they cannot be read.
fn language(data_dir: &Path) -> Option<&'static str> {
    let settings = zajil_core::settings::SettingsStore::new(data_dir)
        .load()
        .ok()?;
    Some(match settings.language {
        zajil_core::settings::Language::Ar => "ar",
        zajil_core::settings::Language::En => "en",
    })
}

fn data_dir() -> Result<PathBuf, String> {
    if let Some(dir) = std::env::var_os("ZAJIL_DATA_DIR") {
        return Ok(PathBuf::from(dir));
    }
    zajil_platform::SystemPaths::new()
        .map(|paths| paths.data_dir().to_path_buf())
        .map_err(|e| e.to_string())
}

/// Hands `request` to the app and returns its answer, as the app wrote it
/// (checked to be a [`Response`]).
fn forward(data_dir: &Path, request: &Request) -> Result<Vec<u8>, String> {
    // A short wait: the app may be busy accepting another connection.
    let mut stream = match zajil_platform::ipc::connect(data_dir, Duration::from_millis(500)) {
        Ok(stream) => stream,
        Err(err) if not_running(&err) => {
            if std::env::var_os("ZAJIL_NO_LAUNCH").is_some() {
                return Err("Zajil is not running".into());
            }
            start_app(data_dir)?;
            zajil_platform::ipc::connect(data_dir, START_PATIENCE)
                .map_err(|e| format!("Zajil was started but does not answer: {e}"))?
        }
        Err(err) => return Err(format!("cannot reach Zajil: {err}")),
    };
    // The talk runs on its own thread, so an app that never answers
    // cannot hold the browser's call: after CHANNEL_TIMEOUT the host
    // answers for it (Windows' pipes have no read timeout of their own).
    let request = request.clone();
    let (done, wait) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        // A request that could not be written never reached the app (it
        // was busy and closed the connection, review of #70): that is not
        // "did not answer", which tells the extension the app may have it.
        let mut talk = || -> Result<Vec<u8>, String> {
            write_json(&mut stream, &request).map_err(|e| format!("cannot reach Zajil: {e}"))?;
            read_frame(&mut stream, MAX_RESPONSE)
                .map_err(|e| format!("Zajil did not answer: {e}"))?
                .ok_or_else(|| "Zajil did not answer: the connection closed".to_owned())
        };
        let _ = done.send(talk());
    });
    let answer = match wait.recv_timeout(CHANNEL_TIMEOUT) {
        Ok(answer) => answer?,
        Err(_) => {
            return Err(format!(
                "Zajil did not answer within {} s",
                CHANNEL_TIMEOUT.as_secs()
            ));
        }
    };
    serde_json::from_slice::<Response>(&answer)
        .map_err(|e| format!("Zajil's answer is not valid: {e}"))?;
    Ok(answer)
}

fn not_running(err: &io::Error) -> bool {
    matches!(
        err.kind(),
        io::ErrorKind::NotFound | io::ErrorKind::ConnectionRefused
    )
}

/// Starts the app installed next to this program, on the same data
/// folder, detached from the browser: no pipes of its, its own process
/// group on macOS and Linux, and on Windows out of the browser's job when
/// the job allows it (Firefox kills its hosts' job when it closes them).
fn start_app(data_dir: &Path) -> Result<(), String> {
    let here = std::env::current_exe().map_err(|e| format!("cannot find myself: {e}"))?;
    let app = here.with_file_name(if cfg!(windows) {
        "zajil-desktop.exe"
    } else {
        "zajil-desktop"
    });
    if !app.is_file() {
        return Err(format!(
            "Zajil is not running, and {} was not found",
            app.display()
        ));
    }
    let command = || {
        let mut command = Command::new(&app);
        command
            .arg("--data-dir")
            .arg(data_dir)
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        #[cfg(unix)]
        std::os::unix::process::CommandExt::process_group(&mut command, 0);
        command
    };
    #[cfg(windows)]
    let spawned = {
        use std::os::windows::process::CommandExt;
        const CREATE_BREAKAWAY_FROM_JOB: u32 = 0x0100_0000;
        const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
        // Out of the browser's job if it allows that; inside it otherwise.
        command()
            .creation_flags(CREATE_BREAKAWAY_FROM_JOB | CREATE_NEW_PROCESS_GROUP)
            .spawn()
            .or_else(|_| command().creation_flags(CREATE_NEW_PROCESS_GROUP).spawn())
    };
    #[cfg(not(windows))]
    let spawned = command().spawn();
    spawned
        .map(drop)
        .map_err(|e| format!("cannot start Zajil: {e}"))
}
