//! The speed limit on the real clock, run every night and never on pull
//! requests (owner's decision on #36, item 14): its failure opens an issue
//! and does not block a merge (`.github/workflows/nightly.yml`).
//!
//! The tests in `speed_limit.rs` run on a `ManualClock`, where the bug of
//! the review of #32 (the first second at 3 to 6 times the limit) shows
//! by a margin of 51 KiB only. Here the margin is wide: the first second
//! may reach twice the limit, the bug gave 5.5 times. Checked by bringing
//! the bug back (each connection's first 256 KiB not counted): 2.4 times
//! the limit with 8 connections, and the test failed. The same downloads
//! also check the average on the real clock, with the owner's tolerance
//! of M3.
//!
//! Not covered here: the line that writes every piece at once under a
//! limit (`limited` in `segmented.rs`). Without it this test still gave
//! 0.97 to 0.997 of the limit, also with segments of 64 KiB (#36).

use std::sync::Mutex;
use std::time::{Duration, Instant};

use zajil_core::{Engine, Event, Options};
use zajil_testserver::{FileSpec, TestServer, random_bytes, sha256_hex};

const MIB: u64 = 1024 * 1024;

#[tokio::test(flavor = "multi_thread")]
#[ignore = "real clock: runs every night (.github/workflows/nightly.yml)"]
async fn on_the_real_clock_the_first_second_stays_under_twice_the_limit() {
    let rate = MIB;
    let data = random_bytes(8 * MIB as usize, 3614);
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(data.clone()));
    for connections in [8, 32] {
        let dir = tempfile::tempdir().unwrap();
        let engine = Engine::new(Options {
            connections,
            ..Options::default()
        })
        .unwrap()
        .with_download_limit(Some(rate));
        // The most on disk within the first second.
        let first_second = Mutex::new(0u64);
        let started = Instant::now();
        let finished = engine
            .download(&server.url("/f.bin"), dir.path(), &mut |event| {
                if let Event::Progress { done, .. } = event
                    && started.elapsed() <= Duration::from_secs(1)
                {
                    let mut most = first_second.lock().unwrap();
                    *most = (*most).max(done);
                }
            })
            .await
            .unwrap();
        let took = started.elapsed();
        assert_eq!(
            sha256_hex(&std::fs::read(&finished.file).unwrap()),
            sha256_hex(&data)
        );
        let most = *first_second.lock().unwrap();
        let speed = data.len() as f64 / took.as_secs_f64() / rate as f64;
        println!(
            "{connections} connections: {most} bytes in the first second, \
             {speed:.3} × the limit over {took:?}"
        );
        assert!(
            most <= 2 * rate,
            "{connections} connections: {most} bytes on disk within the first \
             second, more than twice the limit"
        );
        assert!(
            took >= Duration::from_secs(6),
            "measured over {took:?} only"
        );
        assert!(
            (0.9..=1.1).contains(&speed),
            "{connections} connections: {speed:.3} × the limit over {took:?}"
        );
    }
}
