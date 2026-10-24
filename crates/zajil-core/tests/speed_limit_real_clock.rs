//! The speed limit on the real clock, run every night and never on pull
//! requests (owner's decision on #36, item 14): its failure opens an issue
//! and does not block a merge (`.github/workflows/nightly.yml`).
//!
//! The tests in `speed_limit.rs` run on a `ManualClock`, where the bug of
//! the review of #32 (the first second at 3 to 6 times the limit) shows
//! by a margin of 51 KiB only. Here the first second may reach twice the
//! limit. Checked against the code before the fix (the head of #32, review
//! of #37): 3.0 to 3.4 times the limit in the first second, and the test
//! failed; and against a smaller version of the bug (each connection's
//! first 256 KiB not counted): 2.4 times, failed too. The same downloads
//! also check the average on the real clock, with the owner's tolerance
//! of M3.
//!
//! Not covered here: the line that writes every piece at once under a
//! limit (`limited` in `segmented.rs`). Without it this test still gave
//! 0.97 to 0.997 of the limit, also with segments of 64 KiB (#36).
//!
//! Each download has a time limit, so a limit that waits forever fails the
//! test instead of letting the job time out, which GitHub reports as
//! cancelled rather than failed (review of #37).
//!
//! Measurements (#39, owner's decision): Windows once gave 0.63 of the
//! limit, and about 46 runs on Windows never did again. So the test records
//! every wait the limit asks for (the time asked and the time it took) and
//! every write (its bytes), and the longest stretch without a write. It
//! prints a summary every time; on a failure it also writes the summary and
//! the events as CSV into `ZAJIL_DIAG_DIR` (the nightly job uploads them).
//! The limits it checks are the same as before, and nothing is retried.
//! `ZAJIL_NIGHTLY_FAIL_ON_PURPOSE=1` makes it fail after measuring, to
//! check that the alert and the upload work.

use std::fmt::Write as _;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use zajil_core::limit::{Clock, SystemClock};
use zajil_core::{Engine, Event, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;

/// Every finished wait: when it was asked for, the wait asked for, and
/// the time it took.
type Sleeps = Arc<Mutex<Vec<(Duration, Duration, Duration)>>>;

/// The real clock, recording the waits the limit asks for. A wait that a
/// changed rate cancels is not recorded.
#[derive(Debug)]
struct RecordingClock {
    inner: SystemClock,
    sleeps: Sleeps,
}

impl Clock for RecordingClock {
    fn now(&self) -> Duration {
        self.inner.now()
    }

    fn wall(&self) -> jiff::Timestamp {
        self.inner.wall()
    }

    fn sleep_until(&self, at: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let asked_at = self.inner.now();
        let asked = at.saturating_sub(asked_at);
        let started = Instant::now();
        let sleep = self.inner.sleep_until(at);
        let sleeps = self.sleeps.clone();
        Box::pin(async move {
            sleep.await;
            sleeps
                .lock()
                .unwrap()
                .push((asked_at, asked, started.elapsed()));
        })
    }
}

fn ms(d: Duration) -> f64 {
    d.as_secs_f64() * 1000.0
}

fn percentiles(mut values: Vec<Duration>) -> String {
    values.sort();
    let at = |p: f64| {
        values
            .get(((values.len().max(1) - 1) as f64 * p).round() as usize)
            .copied()
            .unwrap_or_default()
    };
    format!(
        "p50 {:.2} ms, p90 {:.2} ms, max {:.2} ms",
        ms(at(0.5)),
        ms(at(0.9)),
        ms(at(1.0))
    )
}

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real clock: runs every night (.github/workflows/nightly.yml)"]
async fn on_the_real_clock_the_first_second_stays_under_twice_the_limit() {
    let rate = MIB;
    let data = random_bytes(8 * MIB as usize, 3614);
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(data.clone()));
    let mut summary = format!(
        "cores: {}\n",
        std::thread::available_parallelism().map_or(0, |n| n.get())
    );
    let mut csv: Vec<(usize, String)> = Vec::new();
    let mut failures: Vec<String> = Vec::new();
    for connections in [8, 32] {
        let dir = tempfile::tempdir().unwrap();
        let sleeps = Sleeps::default();
        let clock = Arc::new(RecordingClock {
            inner: SystemClock::default(),
            sleeps: sleeps.clone(),
        });
        let engine = Engine::new(Options {
            connections,
            clock: clock.clone(),
            ..Options::default()
        })
        .unwrap()
        .with_download_limit(Some(rate));
        // The most on disk within the first second, and every write.
        let first_second = Mutex::new(0u64);
        let writes = Mutex::new(Vec::<(Duration, u64)>::new());
        let mut last = 0u64;
        let started = Instant::now();
        let url = server.url("/f.bin");
        let mut on_event = |event| {
            if let Event::Progress { done, .. } = event {
                writes
                    .lock()
                    .unwrap()
                    .push((started.elapsed(), done.saturating_sub(last)));
                last = done;
                if started.elapsed() <= Duration::from_secs(1) {
                    let mut most = first_second.lock().unwrap();
                    *most = (*most).max(done);
                }
            }
        };
        let download = engine.download(&url, dir.path(), &mut on_event);
        let result = tokio::time::timeout(Duration::from_secs(120), download).await;
        let took = started.elapsed();
        // A hang, an error or a wrong file is a failure like the others,
        // so its measurements are written too (review of #42: they were
        // lost when these panicked at once).
        let mut downloaded = false;
        match result {
            Err(_) => failures.push(format!("{connections} connections: not done after 120 s")),
            Ok(Err(err)) => {
                failures.push(format!(
                    "{connections} connections: the download failed: {err}"
                ));
            }
            Ok(Ok(finished)) => {
                if sha256_hex(&std::fs::read(&finished.file).unwrap()) == sha256_hex(&data) {
                    downloaded = true;
                } else {
                    failures.push(format!(
                        "{connections} connections: the file is not the original"
                    ));
                }
            }
        }
        let most = *first_second.lock().unwrap();
        let speed = data.len() as f64 / took.as_secs_f64() / rate as f64;

        let sleeps = sleeps.lock().unwrap().clone();
        let writes = writes.lock().unwrap().clone();
        let longest_gap = writes
            .windows(2)
            .map(|w| (w[1].0.saturating_sub(w[0].0), w[0].0))
            .max()
            .unwrap_or_default();
        let _ = writeln!(
            summary,
            "{connections} connections: {most} bytes in the first second, \
             {speed:.3} × the limit over {took:?}\n  \
             waits: {} finished, asked {}, took {}\n  \
             oversleep: {}\n  \
             sum of waits: asked {:.1} ms, took {:.1} ms\n  \
             writes: {}, longest gap {:.1} ms (from {:.1} ms)",
            sleeps.len(),
            percentiles(sleeps.iter().map(|s| s.1).collect()),
            percentiles(sleeps.iter().map(|s| s.2).collect()),
            percentiles(sleeps.iter().map(|s| s.2.saturating_sub(s.1)).collect()),
            ms(sleeps.iter().map(|s| s.1).sum()),
            ms(sleeps.iter().map(|s| s.2).sum()),
            writes.len(),
            ms(longest_gap.0),
            ms(longest_gap.1),
        );
        let mut events = String::from("kind,at_ms,asked_ms,took_ms,bytes\n");
        for (at, asked, took) in &sleeps {
            let _ = writeln!(
                events,
                "sleep,{:.3},{:.3},{:.3},",
                ms(*at),
                ms(*asked),
                ms(*took)
            );
        }
        for (at, bytes) in &writes {
            let _ = writeln!(events, "write,{:.3},,,{bytes}", ms(*at));
        }
        csv.push((connections, events));

        // The checks, unchanged: collected so that both measurements are
        // written before the test fails.
        if !downloaded {
            continue;
        }
        if most > 2 * rate {
            failures.push(format!(
                "{connections} connections: {most} bytes on disk within the first second, \
                 more than twice the limit"
            ));
        }
        if took < Duration::from_secs(6) {
            failures.push(format!("measured over {took:?} only"));
        }
        if !(0.9..=1.1).contains(&speed) {
            failures.push(format!(
                "{connections} connections: {speed:.3} × the limit over {took:?}"
            ));
        }
    }
    if std::env::var_os("ZAJIL_NIGHTLY_FAIL_ON_PURPOSE").is_some() {
        failures.push("failing on purpose (ZAJIL_NIGHTLY_FAIL_ON_PURPOSE)".into());
    }
    print!("{summary}");
    if !failures.is_empty() {
        // Blocking writes in an async test, on purpose: both downloads are
        // over, and nothing else runs (AGENTS.md §10.6).
        if let Some(out) = std::env::var_os("ZAJIL_DIAG_DIR") {
            let out = std::path::PathBuf::from(out);
            std::fs::create_dir_all(&out).unwrap();
            std::fs::write(
                out.join("summary.txt"),
                format!("{summary}failures:\n{}\n", failures.join("\n")),
            )
            .unwrap();
            for (connections, events) in &csv {
                std::fs::write(out.join(format!("events-{connections}.csv")), events).unwrap();
            }
        }
        panic!("{}", failures.join("\n"));
    }
}
