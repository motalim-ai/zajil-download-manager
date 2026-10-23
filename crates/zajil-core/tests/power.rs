//! Shutting down or sleeping after a queue (M3 success criterion:
//! `PowerControl` tested with a mock on the three systems). These tests
//! only ever use [`MockPower`]: no test names or calls the system's real
//! implementation (checked by `zajil-cli/tests/power_guard.rs`).

use std::io;
use std::sync::{Arc, Mutex};
use std::time::Duration;

use tokio::sync::mpsc;
use zajil_core::limit::{Clock, ManualClock};
use zajil_core::queue::{AfterQueue, QueueStore, Runner, Schedule, Status, Summary, after_queue};
use zajil_core::{Engine, Options, Stop};
use zajil_platform::power::{PowerAction, PowerControl};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

/// Records what it is asked for, and does nothing.
#[derive(Default)]
struct MockPower {
    asked: Mutex<Vec<PowerAction>>,
    fail: bool,
}

impl PowerControl for MockPower {
    fn perform(&self, action: PowerAction) -> io::Result<()> {
        self.asked.lock().unwrap().push(action);
        if self.fail {
            Err(io::Error::other("refused by the system"))
        } else {
            Ok(())
        }
    }
}

impl MockPower {
    fn asked(&self) -> Vec<PowerAction> {
        self.asked.lock().unwrap().clone()
    }
}

struct Setup {
    server: TestServer,
    store: QueueStore,
    _dir: tempfile::TempDir,
    downloads: std::path::PathBuf,
}

fn setup() -> Setup {
    let server = TestServer::http();
    server.put("/ok.bin", FileSpec::new(random_bytes(50_000, 900)));
    let dir = tempfile::tempdir().unwrap();
    let downloads = dir.path().join("downloads");
    std::fs::create_dir(&downloads).unwrap();
    Setup {
        server,
        store: QueueStore::new(dir.path()),
        _dir: dir,
        downloads,
    }
}

impl Setup {
    fn add(&self, queue: &str, path: &str) -> u64 {
        let (url, dir) = (self.server.url(path), self.downloads.clone());
        self.store
            .update(move |q| q.add(queue, &url, &dir))
            .unwrap()
    }

    async fn run(&self, until: Option<&str>) -> Summary {
        let engine = Engine::new(Options::default()).unwrap();
        let mut runner = Runner::new(engine, self.store.clone()).poll(Duration::from_millis(20));
        if let Some(queue) = until {
            runner = runner.until_queue_done(queue);
        }
        runner.run(&Stop::new(), &mut |_| {}).await.unwrap()
    }
}

/// Runs [`after_queue`] on a frozen clock in a task, so the test can move
/// the clock and watch the countdown.
fn countdown(
    setup: &Setup,
    queue: &str,
    summary: Summary,
    power: Arc<MockPower>,
    cancel: Stop,
) -> (
    Arc<ManualClock>,
    mpsc::UnboundedReceiver<Duration>,
    tokio::task::JoinHandle<AfterQueue>,
) {
    let clock = ManualClock::frozen("2026-10-23T20:00:00Z".parse().unwrap());
    let (ticks, heard) = mpsc::unbounded_channel();
    let (store, queue, task_clock) = (setup.store.clone(), queue.to_owned(), clock.clone());
    let task = tokio::spawn(async move {
        after_queue(
            &store,
            &queue,
            &summary,
            power.as_ref(),
            PowerAction::Shutdown,
            Duration::from_secs(60),
            task_clock.as_ref(),
            &cancel,
            &mut |left| {
                let _ = ticks.send(left);
            },
        )
        .await
        .unwrap()
    });
    (clock, heard, task)
}

/// The success path: the user asked for a shutdown after queue `night`;
/// it finishes while another queue still waits for its window. The run
/// ends with `night`, a countdown of 60 s is heard second by second, and
/// the system is asked exactly once, at its end.
#[tokio::test]
async fn after_its_queue_finishes_the_computer_is_asked_once_after_the_countdown() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    setup.add("night", "/ok.bin");
    // Another queue with work that would keep the runner going all day.
    setup
        .store
        .update(|q| {
            q.queue_or_new("later")?.schedule = Some(
                Schedule::new(
                    Some("03:00".parse().unwrap()),
                    Some("03:01".parse().unwrap()),
                    vec![],
                )
                .unwrap(),
            );
            Ok(())
        })
        .unwrap();
    setup.add("later", "/ok.bin");

    let summary = setup.run(Some("night")).await;
    assert!(summary.queue_ended && !summary.stopped, "{summary:?}");
    assert_eq!(summary.finished, 2);

    let power = Arc::new(MockPower::default());
    let (clock, mut heard, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    let mut lefts = Vec::new();
    for _ in 0..60 {
        lefts.push(heard.recv().await.unwrap().as_secs());
        assert!(
            power.asked().is_empty(),
            "asked before the end of the countdown"
        );
        clock.advance(Duration::from_secs(1));
    }
    assert!(matches!(task.await.unwrap(), AfterQueue::Done));
    assert_eq!(lefts, (1..=60).rev().collect::<Vec<_>>());
    assert_eq!(power.asked(), [PowerAction::Shutdown]);
    assert_eq!(clock.now(), Duration::from_secs(60));
}

/// Cancelling during the countdown: nothing is asked, ever.
#[tokio::test]
async fn a_cancelled_countdown_asks_nothing() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    let summary = setup.run(Some("night")).await;
    let power = Arc::new(MockPower::default());
    let cancel = Stop::new();
    let (clock, mut heard, task) =
        countdown(&setup, "night", summary, power.clone(), cancel.clone());
    for _ in 0..10 {
        heard.recv().await.unwrap();
        clock.advance(Duration::from_secs(1));
    }
    cancel.request();
    assert!(matches!(task.await.unwrap(), AfterQueue::Cancelled));
    clock.advance(Duration::from_secs(3600));
    assert!(power.asked().is_empty());
}

/// A queue that did not finish (a download failed) asks nothing, and there
/// is no countdown.
#[tokio::test]
async fn a_queue_with_a_failed_download_asks_nothing() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    setup.add("night", "/missing.bin");
    let summary = setup.run(Some("night")).await;
    assert_eq!(summary.failed, 1);
    let power = Arc::new(MockPower::default());
    let (_clock, mut heard, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { left: 1 }
    ));
    assert!(heard.try_recv().is_err(), "a countdown started");
    assert!(power.asked().is_empty());
}

/// Without `until_queue_done`, a run never ends "with a queue": an
/// interface that did not ask for an action cannot get one by mistake.
/// Nor does a run the user stopped, or an empty queue.
#[tokio::test]
async fn nothing_is_asked_unless_the_run_ended_with_that_queue() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    let summary = setup.run(None).await;
    assert!(!summary.queue_ended);
    let power = Arc::new(MockPower::default());
    let (_c, _h, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { .. }
    ));

    let stopped = Summary {
        stopped: true,
        queue_ended: true,
        ..Summary::default()
    };
    let (_c, _h, task) = countdown(&setup, "night", stopped, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { .. }
    ));

    setup
        .store
        .update(|q| q.queue_or_new("empty").map(|_| ()))
        .unwrap();
    let ended = Summary {
        queue_ended: true,
        ..Summary::default()
    };
    let (_c, _h, task) = countdown(&setup, "empty", ended, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { left: 1 }
    ));
    assert!(power.asked().is_empty());
}

/// A paused download in the queue: the run ends with the queue, but it is
/// not finished, so nothing is asked.
#[tokio::test]
async fn a_paused_download_keeps_the_computer_on() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    let paused = setup.add("night", "/ok.bin");
    setup
        .store
        .update(|q| {
            q.item_mut(paused).unwrap().status = Status::Paused;
            Ok(())
        })
        .unwrap();
    let summary = setup.run(Some("night")).await;
    assert!(!summary.queue_ended, "{summary:?}");
    let power = Arc::new(MockPower::default());
    let (_c, _h, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { left: 1 }
    ));
    assert!(power.asked().is_empty());
}

/// Review of #35: a queue whose downloads were all done in an earlier run
/// ended the new run at once, before any other queue downloaded anything,
/// and the computer was asked to shut down. Now the queue must finish a
/// download in this run.
#[tokio::test]
async fn a_queue_finished_in_an_earlier_run_does_not_turn_the_computer_off() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    assert_eq!(setup.run(None).await.finished, 1);
    let other = setup.add("main", "/ok.bin");

    let summary = setup.run(Some("night")).await;
    assert!(!summary.queue_ended, "{summary:?}");
    assert_eq!(summary.finished, 1, "the other queue downloaded");
    assert_eq!(
        setup.store.load().unwrap().item(other).unwrap().status,
        Status::Done
    );
    let power = Arc::new(MockPower::default());
    let (_c, mut heard, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    assert!(matches!(
        task.await.unwrap(),
        AfterQueue::NotFinished { .. }
    ));
    assert!(heard.try_recv().is_err(), "a countdown started");
    assert!(power.asked().is_empty());
}

/// Review of #35: a failed (or paused) download in the chosen queue ended
/// the run and stopped every other download, although nothing would be
/// done. Now the run goes on as if nothing had been asked.
#[tokio::test]
async fn a_failure_in_the_chosen_queue_does_not_stop_other_downloads() {
    let setup = setup();
    setup.server.put(
        "/big.bin",
        FileSpec::new(random_bytes(2 * 1024 * 1024, 901)),
    );
    setup.add("night", "/missing.bin");
    let other = setup.add("main", "/big.bin");

    let summary = setup.run(Some("night")).await;
    assert!(!summary.queue_ended && !summary.stopped, "{summary:?}");
    assert_eq!((summary.finished, summary.failed), (1, 1));
    assert_eq!(
        setup.store.load().unwrap().item(other).unwrap().status,
        Status::Done
    );
}

/// The system may refuse (no permission): the error is reported.
#[tokio::test]
async fn a_refusal_by_the_system_is_reported() {
    let setup = setup();
    setup.add("night", "/ok.bin");
    let summary = setup.run(Some("night")).await;
    let power = Arc::new(MockPower {
        fail: true,
        ..MockPower::default()
    });
    let (clock, mut heard, task) = countdown(&setup, "night", summary, power.clone(), Stop::new());
    while heard.recv().await.is_some() {
        clock.advance(Duration::from_secs(1));
        if clock.now() >= Duration::from_secs(60) {
            break;
        }
    }
    let AfterQueue::Failed(err) = task.await.unwrap() else {
        panic!("expected a failure");
    };
    assert!(err.to_string().contains("refused"), "{err}");
    assert_eq!(power.asked().len(), 1);
}
