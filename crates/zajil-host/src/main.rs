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
//! Nothing is written to stderr that the browser would need: Chrome keeps
//! it in its log, Firefox in the browser console.

use std::io;
use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode, Stdio};
use std::time::Duration;

use zajil_host::{
    FrameError, MAX_REQUEST, MAX_RESPONSE, Request, Response, parse, read_frame, write_frame,
    write_json,
};
use zajil_platform::AppPaths;

/// How long to wait for an app that is being started.
const START_PATIENCE: Duration = Duration::from_secs(20);

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
            Ok(Request::Ping { .. }) => write_json(&mut output, &Response::ok()),
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
    let mut stream = match zajil_platform::ipc::connect(data_dir, Duration::ZERO) {
        Ok(stream) => stream,
        Err(err) if not_running(&err) => {
            if std::env::var_os("ZAJIL_NO_LAUNCH").is_some() {
                return Err("Zajil is not running".into());
            }
            start_app()?;
            zajil_platform::ipc::connect(data_dir, START_PATIENCE)
                .map_err(|e| format!("Zajil was started but does not answer: {e}"))?
        }
        Err(err) => return Err(format!("cannot reach Zajil: {err}")),
    };
    let talk = |stream: &mut zajil_platform::ipc::Stream| -> io::Result<Vec<u8>> {
        write_json(stream, request)?;
        read_frame(stream, MAX_RESPONSE)
            .map_err(|e| io::Error::new(io::ErrorKind::InvalidData, e.to_string()))?
            .ok_or_else(|| io::Error::from(io::ErrorKind::UnexpectedEof))
    };
    let answer = talk(&mut stream).map_err(|e| format!("Zajil did not answer: {e}"))?;
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

/// Starts the app installed next to this program, detached from the
/// browser's pipes.
fn start_app() -> Result<(), String> {
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
    Command::new(&app)
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map(drop)
        .map_err(|e| format!("cannot start Zajil: {e}"))
}
