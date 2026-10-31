//! How Zajil stops: the downloads record their progress first, on a
//! request to exit, a stop signal, or the end of a Windows session.

#[cfg(windows)]
use std::sync::atomic::Ordering;
use std::time::Duration;

#[cfg(windows)]
use tauri::Manager;

#[cfg(windows)]
use crate::{App, saving};

/// How long the program waits for the downloads to record their progress
/// when it is closed.
pub(crate) const STOP_WAIT: Duration = Duration::from_secs(5);
/// At the end of a Windows session, how long Zajil waits at most for its
/// downloads to stop and its writes to end before ending its process
/// (#81). Usually far less (the downloads stopped within 0.2 s in #81's
/// runs); past it, a write is not waited for: every file is replaced in
/// one step (ADR-0004), so the one before stays whole.
#[cfg(windows)]
const SESSION_END_WAIT: Duration = Duration::from_secs(10);

/// Waits for the queues to stop, `limit` at most, says how it ended, and
/// whether they stopped. After that it gives up: every download's progress
/// is in its state file, at most a second old (ADR-0004), and the next
/// start resumes there.
pub(crate) async fn wait_for_queues(
    mut stopped: tokio::sync::watch::Receiver<bool>,
    limit: Duration,
) -> bool {
    // How long the stop took, said every time (#132: on Windows it ran
    // out a few times; the time shows how near the limit it usually is).
    let asked = std::time::Instant::now();
    let wait = stopped.wait_for(|stopped| *stopped);
    let ended = tokio::time::timeout(limit, wait).await.is_ok();
    if ended {
        eprintln!(
            "zajil: the downloads stopped; exiting (after {} ms)",
            asked.elapsed().as_millis()
        );
    } else {
        eprintln!(
            "zajil: the downloads did not stop within {} s; exiting, to resume from their state files (after {} ms)",
            limit.as_secs(),
            asked.elapsed().as_millis()
        );
    }
    ended
}

/// The signals that ask a program to stop (SIGTERM: a shutdown on Linux,
/// `kill`; SIGINT: Ctrl+C in a terminal; SIGHUP: its terminal closed). On
/// Windows the system closes the window, or ends the session
/// (`on_windows_session_end`, #49).
#[cfg(unix)]
pub(crate) struct StopSignals {
    terminate: tokio::signal::unix::Signal,
    interrupt: tokio::signal::unix::Signal,
    hangup: tokio::signal::unix::Signal,
}

/// Registered before the window opens: a SIGTERM that comes early still
/// closes the program cleanly (one came before the handler was ready in
/// CI, on #46, and killed it). `None`, said on stderr, if the system
/// refuses.
#[cfg(unix)]
pub(crate) fn stop_signals() -> Option<StopSignals> {
    use tokio::signal::unix::{SignalKind, signal};
    tauri::async_runtime::block_on(async {
        Ok::<_, std::io::Error>(StopSignals {
            terminate: signal(SignalKind::terminate())?,
            interrupt: signal(SignalKind::interrupt())?,
            hangup: signal(SignalKind::hangup())?,
        })
    })
    .inspect_err(|err| eprintln!("zajil: signals will not close the program cleanly: {err}"))
    .ok()
}

/// Exits as when the window is closed, so the downloads record their
/// progress, and with code 0. A second signal while it waits exits at once
/// (review of #45: it was ignored), with code 1: the progress of the last
/// second may be lost, never a file.
#[cfg(unix)]
pub(crate) async fn exit_on_signal(handle: tauri::AppHandle, mut signals: StopSignals) {
    let mut asked = false;
    loop {
        let name = tokio::select! {
            _ = signals.terminate.recv() => "SIGTERM",
            _ = signals.interrupt.recv() => "SIGINT",
            _ = signals.hangup.recv() => "SIGHUP",
        };
        if asked {
            eprintln!("zajil: asked again ({name}); exiting now");
            std::process::exit(1);
        }
        asked = true;
        eprintln!("zajil: asked to stop ({name})");
        handle.exit(0);
    }
}

/// Windows has no SIGTERM: at a shutdown, restart or logoff it sends the
/// window `WM_ENDSESSION`, and may end the process once that is answered
/// (#49). The downloads stop there, after recording their progress, as
/// when the window is closed; then, once nothing is being written, the
/// process is ended at once: its usual exit waited seconds for WebView2
/// (#81). Only here: the other ways of quitting exit as before.
#[cfg(windows)]
pub(crate) fn on_windows_session_end(tauri_app: &tauri::App) -> tauri::Result<()> {
    let Some(window) = tauri_app.get_webview_window("main") else {
        return Ok(());
    };
    let handle = tauri_app.handle().clone();
    let hwnd = window.hwnd()?.0 as isize;
    zajil_platform::session::on_session_end(
        hwnd,
        Box::new(move || {
            eprintln!("zajil: asked to stop (the Windows session is ending)");
            let app = handle.state::<App>();
            if !app.exiting.swap(true, Ordering::SeqCst) {
                app.stop.request();
            }
            let asked = std::time::Instant::now();
            let stopped = tauri::async_runtime::block_on(wait_for_queues(
                app.stopped.subscribe(),
                SESSION_END_WAIT,
            ));
            // Kept until the end: no write starts after.
            let (_held, late) = saving::hold(&app.data_dir, asked + SESSION_END_WAIT);
            let still: Vec<_> = (!stopped).then_some("downloads").into_iter().chain(late).collect();
            if still.is_empty() {
                eprintln!("zajil: saved; terminating to finish the session end");
            } else {
                eprintln!(
                    "zajil: not everything saved within {} s (still writing: {}); terminating to finish the session end",
                    SESSION_END_WAIT.as_secs(),
                    still.join(", ")
                );
            }
            zajil_platform::session::terminate();
        }),
    )
    .map_err(tauri::Error::Io)
}
