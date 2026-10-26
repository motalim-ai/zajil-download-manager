//! `zajil batch`: download the files a pattern stands for, one after the
//! other (M7, ADR-0010 section 8).

use std::path::{Path, PathBuf};
use std::process::ExitCode;
use std::time::Duration;

use zajil_core::{Engine, Error, Event, Finished, Stop, Url};
use zajil_grab::pattern::{self, PatternError, Star};

use crate::{Progress, download_dir, human_bytes, stopped};

/// Exit code of a pattern that was refused: wrong usage, like clap's own.
const EXIT_USAGE: u8 = 2;

#[derive(Debug, clap::Args)]
pub struct BatchArgs {
    /// The URL with ranges and lists: `[001-100]` (zeros give the width),
    /// `[a-z]`, `[0-100:5]` (a step), `{jan,feb}`, or `*` with --from and
    /// --to. In the path and query only. `\[`, `\]`, `\{`, `\}`, `\*`, `\,`
    /// are the characters themselves; write a backslash as %5C.
    pattern: String,
    /// Where the `*` counter starts: a number or a letter.
    #[arg(long, value_name = "START", requires = "to")]
    from: Option<String>,
    /// Where the `*` counter ends: a number or a letter.
    #[arg(long, value_name = "END", requires = "from")]
    to: Option<String>,
    /// Digits of each `*` number, with zeros on the left: 3 gives 001.
    #[arg(long, value_name = "N", requires = "from")]
    width: Option<u8>,
    /// Most URLs the pattern may make.
    #[arg(
        long,
        value_name = "N",
        default_value_t = pattern::DEFAULT_MAX_URLS,
        value_parser = clap::value_parser!(u64).range(1..=pattern::MAX_URLS)
    )]
    max: u64,
    /// Folder to save into. Without it, the system Downloads folder.
    #[arg(short, long, value_name = "FOLDER")]
    dir: Option<PathBuf>,
    /// Only print the URLs, one per line, without downloading anything.
    #[arg(long)]
    print: bool,
}

pub async fn run(engine: Engine, args: BatchArgs) -> ExitCode {
    let star = args.from.map(|from| Star {
        from,
        to: args.to.unwrap_or_default(),
        width: args.width,
    });
    let urls = match pattern::expand(&args.pattern, star.as_ref(), args.max) {
        Ok(urls) => urls,
        Err(err) => return refused(&err),
    };
    if args.print {
        // The whole URL, query included: the user wrote it and patterns may
        // be in the query (ADR-0010, section 8). Never a password.
        for url in &urls {
            println!("{}", without_userinfo(url));
        }
        return ExitCode::SUCCESS;
    }
    let dir = match download_dir(args.dir.as_deref()) {
        Ok(dir) => dir,
        Err(message) => return crate::fail(&message),
    };

    let stop = Stop::new();
    let on_ctrl_c = stop.clone();
    tokio::spawn(async move {
        if tokio::signal::ctrl_c().await.is_ok() {
            on_ctrl_c.request();
        }
    });

    let total = urls.len();
    let outcome = download_all(&engine, &urls, &dir, &stop, &mut |_, _| {}).await;
    match outcome {
        Outcome::Done { saved, failed } => {
            summary(saved, failed, total);
            if failed == 0 {
                ExitCode::SUCCESS
            } else {
                ExitCode::FAILURE
            }
        }
        Outcome::Stopped {
            at,
            saved,
            failed,
            state_file,
        } => {
            summary(saved, failed, total);
            eprintln!("zajil: stopped at {at}/{total}");
            match state_file {
                Some(state_file) => stopped("cancelled", &state_file),
                None => crate::fail("cancelled"),
            }
        }
        Outcome::Busy {
            at,
            saved,
            failed,
            wait,
        } => {
            summary(saved, failed, total);
            crate::fail(&format!(
                "the server asks to wait {} s before more requests; stopped at {at}/{total}",
                wait.as_secs()
            ))
        }
    }
}

/// How a batch ended.
#[derive(Debug, PartialEq, Eq)]
enum Outcome {
    /// Every URL was tried.
    Done { saved: usize, failed: usize },
    /// Stopped (Ctrl+C) at URL `at` (counted from 1); `state_file` resumes
    /// it when some of it was written.
    Stopped {
        at: usize,
        saved: usize,
        failed: usize,
        state_file: Option<PathBuf>,
    },
    /// The server asked to wait longer than Zajil waits, at URL `at`: the
    /// next URLs are on the same server, so the batch stops there.
    Busy {
        at: usize,
        saved: usize,
        failed: usize,
        wait: Duration,
    },
}

/// Downloads `urls` into `dir` one after the other, until `stop`. `watch`
/// sees each engine event with the index of its URL (tests use it).
async fn download_all(
    engine: &Engine,
    urls: &[String],
    dir: &Path,
    stop: &Stop,
    watch: &mut (dyn FnMut(usize, &Event) + Send),
) -> Outcome {
    let total = urls.len();
    let (mut saved, mut failed) = (0usize, 0usize);
    for (n, url) in urls.iter().enumerate() {
        if stop.is_requested() {
            return Outcome::Stopped {
                at: n + 1,
                saved,
                failed,
                state_file: None,
            };
        }
        eprintln!("zajil: [{}/{total}] {}", n + 1, shown(url));
        let mut progress = Progress::new();
        let result = engine
            .download_until(url, dir, stop, &mut |e| {
                watch(n, &e);
                progress.show(e)
            })
            .await;
        progress.clear_line();
        match result {
            Ok(Finished { file, bytes }) => {
                saved += 1;
                println!("Saved {} ({})", file.display(), human_bytes(bytes));
            }
            Err(Error::Stopped { state_file }) => {
                return Outcome::Stopped {
                    at: n + 1,
                    saved,
                    failed,
                    state_file,
                };
            }
            Err(Error::ServerBusy { wait }) => {
                return Outcome::Busy {
                    at: n + 1,
                    saved,
                    failed,
                    wait,
                };
            }
            Err(Error::Interrupted { state_file, reason }) => {
                failed += 1;
                // Resumable on its own: say how, and go on with the next.
                let _ = stopped(&reason, &state_file);
            }
            Err(err) => {
                failed += 1;
                eprintln!("zajil: [{}/{total}] failed: {err}", n + 1);
            }
        }
    }
    Outcome::Done { saved, failed }
}

fn refused(err: &PatternError) -> ExitCode {
    eprintln!("zajil: error: {err}");
    ExitCode::from(EXIT_USAGE)
}

fn summary(saved: usize, failed: usize, total: usize) {
    eprintln!("zajil: {saved} saved, {failed} failed, of {total}");
}

/// `url` as it may be printed: without a user name, password, query or
/// fragment, which may hold a token (owner's decision on #28).
pub(crate) fn shown(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut url) => {
            url.set_query(None);
            url.set_fragment(None);
            without_userinfo(url.as_str())
        }
        Err(_) => String::new(),
    }
}

/// `url` without a user name and password (AGENTS.md §10.3).
fn without_userinfo(url: &str) -> String {
    match Url::parse(url) {
        Ok(mut url) => {
            let _ = url.set_username("");
            let _ = url.set_password(None);
            url.into()
        }
        Err(_) => String::new(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::Options;
    use zajil_testserver::{Failure, FileSpec, TestServer, random_bytes, sha256_hex};

    fn engine() -> Engine {
        Engine::new(Options {
            connections: 1,
            ..Options::default()
        })
        .unwrap()
    }

    /// AGENTS.md §10.5: stopping in the middle of a batch (what Ctrl+C
    /// does) ends the file being downloaded so it can be resumed, never
    /// asks for the files after it, and leaves nothing else behind.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_in_the_middle_of_a_batch_leaves_a_resumable_file_and_nothing_else() {
        let server = TestServer::http();
        let second = random_bytes(2_000_000, 2);
        server.put("/f1.bin", FileSpec::new(b"one".to_vec()));
        server.put("/f2.bin", FileSpec::new(second.clone()).throttle(200_000));
        server.put("/f3.bin", FileSpec::new(b"three".to_vec()));
        let dir = tempfile::tempdir().unwrap();
        let urls: Vec<String> = ["/f1.bin", "/f2.bin", "/f3.bin"]
            .iter()
            .map(|p| server.url(p))
            .collect();

        let stop = Stop::new();
        let on_progress = stop.clone();
        let outcome = download_all(&engine(), &urls, dir.path(), &stop, &mut |n, e| {
            if n == 1 && matches!(e, Event::Progress { done, .. } if *done > 0) {
                on_progress.request();
            }
        })
        .await;

        let Outcome::Stopped {
            at: 2,
            saved: 1,
            failed: 0,
            state_file: Some(state_file),
        } = outcome
        else {
            panic!("{outcome:?}");
        };
        assert!(
            server.gets("/f3.bin").is_empty(),
            "the third file was asked for"
        );
        let mut left: Vec<String> = std::fs::read_dir(dir.path())
            .unwrap()
            .map(|e| e.unwrap().file_name().to_string_lossy().into_owned())
            .collect();
        left.sort();
        assert_eq!(left, ["f1.bin", "f2.bin.zajil-part", "f2.bin.zajil-state"]);

        // The stopped file resumes to the right content (served at full
        // speed now: the same bytes, so the same validator).
        server.put("/f2.bin", FileSpec::new(second.clone()));
        let done = engine()
            .resume(&state_file, &mut |_| {})
            .await
            .expect("resume");
        assert_eq!(
            sha256_hex(&std::fs::read(done.file).unwrap()),
            sha256_hex(&second)
        );
    }

    /// A stop between two files: the next one is never asked for.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_stop_between_files_asks_for_nothing_more() {
        let server = TestServer::http();
        server.put("/a.bin", FileSpec::new(b"a".to_vec()));
        server.put("/b.bin", FileSpec::new(b"b".to_vec()));
        let dir = tempfile::tempdir().unwrap();
        let urls = vec![server.url("/a.bin"), server.url("/b.bin")];
        let stop = Stop::new();
        let on_done = stop.clone();
        let outcome = download_all(&engine(), &urls, dir.path(), &stop, &mut |n, e| {
            if n == 0 && matches!(e, Event::Finished { .. }) {
                on_done.request();
            }
        })
        .await;
        assert_eq!(
            outcome,
            Outcome::Stopped {
                at: 2,
                saved: 1,
                failed: 0,
                state_file: None
            }
        );
        assert!(server.gets("/b.bin").is_empty());
    }

    /// A server that asks to wait longer than Zajil waits: the batch stops
    /// there instead of asking the same server for the next files.
    #[tokio::test(flavor = "multi_thread")]
    async fn a_server_that_asks_to_wait_stops_the_batch() {
        let server = TestServer::http();
        let busy = Failure {
            status: 429,
            retry_after: Some(3600),
        };
        server.put("/a.bin", FileSpec::new(b"a".to_vec()));
        server.put(
            "/b.bin",
            FileSpec::new(b"b".to_vec()).fail_first(vec![busy; 5]),
        );
        server.put("/c.bin", FileSpec::new(b"c".to_vec()));
        let dir = tempfile::tempdir().unwrap();
        let urls: Vec<String> = ["/a.bin", "/b.bin", "/c.bin"]
            .iter()
            .map(|p| server.url(p))
            .collect();
        let outcome =
            download_all(&engine(), &urls, dir.path(), &Stop::new(), &mut |_, _| {}).await;
        assert_eq!(
            outcome,
            Outcome::Busy {
                at: 2,
                saved: 1,
                failed: 0,
                wait: Duration::from_secs(3600)
            }
        );
        assert!(server.gets("/c.bin").is_empty());
    }

    #[test]
    fn printed_urls_never_carry_a_password() {
        assert_eq!(
            without_userinfo("https://u:pw@h/a?sig=1"),
            "https://h/a?sig=1"
        );
        assert_eq!(shown("https://u:pw@h/a?sig=1#x"), "https://h/a");
    }
}
