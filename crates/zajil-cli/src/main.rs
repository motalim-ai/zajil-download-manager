//! `zajil`: download and resume from the command line.
//!
//! A thin layer over `zajil-core`: it parses arguments, shows the engine's
//! events and maps the result to an exit code. There is deliberately no
//! option that weakens TLS certificate checks.
//!
//! Messages are in English until the translation system arrives with the
//! GUI (M4, ADR-0003).

#![forbid(unsafe_code)]

mod queue;

use std::io::{IsTerminal, Write};
use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::{Duration, Instant};

use clap::{Parser, Subcommand};
use zajil_core::{
    DEFAULT_CONNECTIONS, DEFAULT_TRIES, Engine, Error, Event, Finished, MAX_CONNECTIONS, Options,
    RestartReason,
};
use zajil_platform::shell::{self, ShellCommand};
use zajil_platform::{AppPaths, SystemPaths};

/// Exit code when the download stopped but can be resumed.
const EXIT_RESUMABLE: u8 = 3;

#[derive(Debug, Parser)]
#[command(
    name = "zajil",
    version,
    about = "Zajil Download Manager — command line",
    after_help = "Exit codes: 0 done, 1 failed, 2 wrong usage, 3 stopped but resumable \
                  (run `zajil resume <state-file>`)."
)]
struct Cli {
    #[command(subcommand)]
    command: Command,
    /// Connections for the download, 1 to 32. A file is split only if its
    /// server allows it.
    #[arg(
        short,
        long,
        global = true,
        value_name = "N",
        default_value_t = DEFAULT_CONNECTIONS as u16,
        value_parser = clap::value_parser!(u16).range(1..=MAX_CONNECTIONS as i64)
    )]
    connections: u16,
    /// How many times in a row a failing request or connection is tried
    /// before the download stops (it can be resumed later). 1 means no
    /// retry. The waits in between grow, and a server's Retry-After is
    /// respected.
    #[arg(
        long,
        global = true,
        value_name = "N",
        default_value_t = DEFAULT_TRIES,
        value_parser = clap::value_parser!(u32).range(1..=100)
    )]
    tries: u32,
    /// Where Zajil keeps its queues. Defaults to the system data folder.
    #[arg(long, global = true, value_name = "FOLDER")]
    data_dir: Option<PathBuf>,
}

#[derive(Debug, Subcommand)]
enum Command {
    /// Download a file over HTTP or HTTPS.
    Get {
        /// The http:// or https:// URL to download.
        url: String,
        /// Folder to save into. Defaults to the system Downloads folder.
        #[arg(short, long, value_name = "FOLDER")]
        dir: Option<PathBuf>,
        /// Speed limit, in bytes per second, or with K, M or G (KiB/s,
        /// MiB/s, GiB/s), such as 500K or 1.5M.
        #[arg(long, value_name = "RATE", value_parser = rate)]
        limit: Option<u64>,
    },
    /// Continue an interrupted download from its .zajil-state file.
    Resume {
        /// The `<name>.zajil-state` file next to the partial download.
        state_file: PathBuf,
        /// Speed limit, like `zajil get --limit`.
        #[arg(long, value_name = "RATE", value_parser = rate)]
        limit: Option<u64>,
    },
    /// Manage and run download queues.
    Queue {
        #[command(subcommand)]
        command: queue::QueueCommand,
    },
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if let Command::Queue { command } = cli.command {
        let store = match queue_store(cli.data_dir.as_deref()) {
            Ok(store) => store,
            Err(message) => return fail(&message),
        };
        let queue::QueueCommand::Run {
            shutdown_after,
            sleep_after,
            countdown,
        } = command
        else {
            return queue::manage(&store, command);
        };
        let power = queue::PowerRequest::from_flags(shutdown_after, sleep_after, countdown);
        let runtime = match tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()
        {
            Ok(runtime) => runtime,
            Err(err) => return fail(&format!("could not start: {err}")),
        };
        let engine = match Engine::new(options(cli.connections.into(), cli.tries)) {
            Ok(engine) => engine,
            Err(err) => return fail(&err.to_string()),
        };
        return runtime.block_on(queue::run(store, engine, power));
    }
    let runtime = match tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
    {
        Ok(runtime) => runtime,
        Err(err) => return fail(&format!("could not start: {err}")),
    };
    runtime.block_on(run(cli.command, cli.connections.into(), cli.tries))
}

/// A speed limit given on the command line (`zajil_core::limit::parse_rate`).
fn rate(text: &str) -> Result<u64, String> {
    zajil_core::limit::parse_rate(text).ok_or_else(|| {
        format!(
            "{text:?} is not a speed: use bytes per second, or K, M or G (such as 500K or 1.5M)"
        )
    })
}

fn options(connections: usize, tries: u32) -> Options {
    Options {
        connections,
        max_failures: tries,
        ..Options::default()
    }
}

/// The queues file, in `--data-dir` or the system data folder.
fn queue_store(chosen: Option<&Path>) -> Result<zajil_core::queue::QueueStore, String> {
    let dir = match chosen {
        Some(dir) => dir.to_path_buf(),
        None => SystemPaths::new()
            .map_err(|err| err.to_string())?
            .data_dir()
            .to_path_buf(),
    };
    Ok(zajil_core::queue::QueueStore::new(dir))
}

async fn run(command: Command, connections: usize, tries: u32) -> ExitCode {
    let engine = match Engine::new(options(connections, tries)) {
        Ok(engine) => engine,
        Err(err) => return fail(&err.to_string()),
    };
    let mut progress = Progress::new();

    let work = async {
        match &command {
            Command::Get { url, dir, limit } => {
                let dir = match download_dir(dir.as_deref()) {
                    Ok(dir) => dir,
                    Err(message) => return Err(Failure::Message(message)),
                };
                engine
                    .with_download_limit(*limit)
                    .download(url, &dir, &mut |e| progress.show(e))
                    .await
                    .map_err(Failure::Engine)
            }
            Command::Resume { state_file, limit } => engine
                .with_download_limit(*limit)
                .resume(state_file, &mut |e| progress.show(e))
                .await
                .map_err(Failure::Engine),
            Command::Queue { .. } => unreachable!("handled in main"),
        }
    };

    let result = tokio::select! {
        result = work => result,
        _ = tokio::signal::ctrl_c() => Err(Failure::Cancelled),
    };
    progress.clear_line();

    match result {
        Ok(Finished { file, bytes }) => {
            println!("Saved {} ({})", file.display(), human_bytes(bytes));
            ExitCode::SUCCESS
        }
        Err(Failure::Engine(Error::Interrupted { state_file, reason })) => {
            stopped(&reason, &state_file)
        }
        Err(Failure::Engine(err @ Error::CredentialsNeeded { .. })) => needs_credentials(&err),
        Err(Failure::Cancelled) => match progress.state_file.clone() {
            Some(state_file) => stopped("cancelled", &state_file),
            None => fail("cancelled"),
        },
        Err(Failure::Engine(err)) => fail(&err.to_string()),
        Err(Failure::Message(message)) => fail(&message),
    }
}

enum Failure {
    Engine(Error),
    Message(String),
    Cancelled,
}

/// The folder to save into, created if needed.
fn download_dir(chosen: Option<&Path>) -> Result<PathBuf, String> {
    let dir = match chosen {
        Some(dir) => dir.to_path_buf(),
        None => SystemPaths::new()
            .map_err(|err| err.to_string())?
            .download_dir()
            .to_path_buf(),
    };
    std::fs::create_dir_all(&dir)
        .map_err(|err| format!("cannot use the folder {}: {err}", dir.display()))?;
    Ok(dir)
}

fn stopped(reason: &str, state_file: &Path) -> ExitCode {
    eprintln!("zajil: the download stopped: {reason}");
    // The file name comes from the server: quote it for each shell so the
    // printed command is safe to copy and run.
    let arg = state_file.to_string_lossy();
    for ShellCommand { shell, command } in shell::commands("zajil resume", &arg) {
        match command {
            Some(command) => {
                eprintln!("zajil: to resume, run this in {shell}:");
                eprintln!("    {command}");
            }
            None => {
                eprintln!("zajil: {shell} cannot be given this path safely; use another shell.")
            }
        }
    }
    ExitCode::from(EXIT_RESUMABLE)
}

/// A resume the server refused with 401: the state file never keeps a user
/// name or password, so tell the user how to download the file again.
///
/// The user runs their own `zajil get` again (no command with the URL is
/// printed: it would put the password in a command line and show a token
/// from the URL), after deleting the two old files. Those paths come from
/// the server, so the delete command and the folder are quoted for each
/// shell, like the resume command in [`stopped`] (review of #29).
fn needs_credentials(err: &Error) -> ExitCode {
    let Error::CredentialsNeeded { state_file, .. } = err else {
        return fail(&err.to_string());
    };
    eprintln!("zajil: error: {err}");
    let part_file = zajil_core::state::part_path(
        &zajil_core::state::final_path_for_state(state_file).unwrap_or_default(),
    );
    let (state_arg, part_arg) = (state_file.to_string_lossy(), part_file.to_string_lossy());
    let dir = state_file.parent().unwrap_or(Path::new("."));
    eprintln!("zajil: to download it again:");
    eprintln!("zajil:   1. delete the two files this download left:");
    for ShellCommand { shell, command } in shell::delete_commands(&[&state_arg, &part_arg]) {
        match command {
            Some(command) => {
                eprintln!("zajil:      in {shell}:");
                eprintln!("    {command}");
            }
            None => eprintln!(
                "zajil:      {shell} cannot be given these paths safely; use another shell."
            ),
        }
    }
    eprintln!(
        "zajil:   2. run the same `zajil get` command again, with your user name and password \
         in the URL, and with this folder:"
    );
    for ShellCommand { shell, command } in shell::commands("--dir", &dir.to_string_lossy()) {
        match command {
            Some(command) => {
                eprintln!("zajil:      in {shell}:");
                eprintln!("    {command}");
            }
            None => eprintln!(
                "zajil:      {shell} cannot be given this folder safely; use another shell."
            ),
        }
    }
    ExitCode::FAILURE
}

fn fail(message: &str) -> ExitCode {
    eprintln!("zajil: error: {message}");
    ExitCode::FAILURE
}

/// Shows engine events on stderr: a live line on a terminal, and only the
/// important events otherwise.
struct Progress {
    live: bool,
    /// The terminal understands ANSI escape codes.
    ansi: bool,
    /// Characters of the last progress line, to overwrite it without them.
    width: usize,
    last_draw: Option<Instant>,
    started: Instant,
    from: u64,
    state_file: Option<PathBuf>,
}

impl Progress {
    fn new() -> Self {
        let live = std::io::stderr().is_terminal();
        Progress {
            live,
            ansi: live && zajil_platform::console::enable_ansi_on_stderr(),
            width: 0,
            last_draw: None,
            started: Instant::now(),
            from: 0,
            state_file: None,
        }
    }

    fn show(&mut self, event: Event) {
        match event {
            Event::Started {
                file,
                state_file,
                total,
                from,
            } => {
                self.clear_line();
                self.started = Instant::now();
                self.from = from;
                self.state_file = Some(state_file);
                let size = total
                    .map(human_bytes)
                    .unwrap_or_else(|| "unknown size".into());
                if from > 0 {
                    eprintln!(
                        "Resuming {} at {} ({size})",
                        file.display(),
                        human_bytes(from)
                    );
                } else {
                    eprintln!("Downloading {} ({size})", file.display());
                }
            }
            Event::Progress { done, total } => self.draw(done, total),
            Event::Restarted { reason } => {
                self.clear_line();
                eprintln!("Starting over: {}", describe(reason));
            }
            Event::ServerBusy {
                status,
                connections,
                wait,
            } => {
                self.clear_line();
                let wait = wait
                    .map(|w| format!(", waiting {} s", w.as_secs_f64().ceil()))
                    .unwrap_or_default();
                eprintln!(
                    "The server answered {status}: {connections} connection(s) from now on{wait}"
                );
            }
            Event::Retrying { wait, reason } => {
                self.clear_line();
                eprintln!("Trying again in {} s: {reason}", wait.as_secs_f64().ceil());
            }
            Event::Split { .. } | Event::Finished { .. } => {}
        }
    }

    fn draw(&mut self, done: u64, total: Option<u64>) {
        if !self.live
            || self
                .last_draw
                .is_some_and(|t| t.elapsed() < Duration::from_millis(100))
        {
            return;
        }
        self.last_draw = Some(Instant::now());
        let secs = self.started.elapsed().as_secs_f64().max(0.001);
        let speed = human_bytes(((done - self.from.min(done)) as f64 / secs) as u64);
        let line = match total {
            Some(total) if total > 0 => format!(
                "{:5.1}%  {} / {}  {speed}/s",
                done as f64 * 100.0 / total as f64,
                human_bytes(done),
                human_bytes(total)
            ),
            _ => format!("{}  {speed}/s", human_bytes(done)),
        };
        let mut err = std::io::stderr();
        let _ = write!(err, "{}", redraw(&line, self.width, self.ansi));
        let _ = err.flush();
        self.width = line.chars().count();
    }

    fn clear_line(&mut self) {
        if self.live && self.last_draw.take().is_some() {
            let _ = write!(std::io::stderr(), "{}", redraw("", self.width, self.ansi));
            let _ = write!(std::io::stderr(), "\r");
            self.width = 0;
        }
    }
}

/// What to write to replace the previous progress line (`previous_width`
/// characters) with `line`. Without ANSI support (the older Windows
/// console), the old line is overwritten with spaces instead of an escape
/// code, so no `←[2K` garbage appears.
fn redraw(line: &str, previous_width: usize, ansi: bool) -> String {
    if ansi {
        format!("\r\x1b[2K{line}")
    } else {
        let padding = previous_width.saturating_sub(line.chars().count());
        format!("\r{line}{}", " ".repeat(padding))
    }
}

fn describe(reason: RestartReason) -> &'static str {
    match reason {
        RestartReason::FileChanged => "the file changed on the server",
        RestartReason::RangesIgnored => "the server cannot continue partial downloads",
        RestartReason::RangeMismatch => "the server sent a different part than requested",
        RestartReason::NoValidator => {
            "the server gives no reliable way to detect a changed file (no strong ETag or date)"
        }
        RestartReason::NothingDone => "nothing was downloaded yet",
        RestartReason::PartFileMissing => "the .zajil-part file is missing",
        RestartReason::PartFileTooShort => "the .zajil-part file is shorter than recorded",
    }
}

fn human_bytes(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KiB", "MiB", "GiB", "TiB"];
    let mut value = bytes as f64;
    let mut unit = 0;
    while value >= 1024.0 && unit < UNITS.len() - 1 {
        value /= 1024.0;
        unit += 1;
    }
    if unit == 0 {
        format!("{bytes} B")
    } else {
        format!("{value:.1} {}", UNITS[unit])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formats_sizes() {
        assert_eq!(human_bytes(0), "0 B");
        assert_eq!(human_bytes(1023), "1023 B");
        assert_eq!(human_bytes(1536), "1.5 KiB");
        assert_eq!(human_bytes(50 * 1024 * 1024), "50.0 MiB");
        assert_eq!(human_bytes(5 * 1024 * 1024 * 1024), "5.0 GiB");
    }

    #[test]
    fn redraws_without_escape_codes_when_the_terminal_lacks_them() {
        let plain = redraw("  5.0%", 12, false);
        assert!(!plain.contains('\x1b'), "{plain:?}");
        assert_eq!(plain, format!("\r  5.0%{}", " ".repeat(6)));
        assert_eq!(redraw("abc", 0, false), "\rabc");
        assert_eq!(redraw("abc", 2, true), "\r\x1b[2Kabc");
    }

    #[test]
    fn arguments_parse() {
        use clap::CommandFactory;
        Cli::command().debug_assert();
    }
}
