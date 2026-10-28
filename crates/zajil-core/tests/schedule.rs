//! Schedules (M3), on a clock the test owns (owner's decision on the M3
//! plan): a frozen `ManualClock` only moves when the test says, so "it is
//! now 23:00" is exact and does not depend on how fast CI is. The one test
//! on the real clock is in `zajil-cli/tests/queue.rs`.
//!
//! Owner's tolerance (ROADMAP M3): a download scheduled 5 seconds ahead
//! starts within one second of its time.

use std::sync::Arc;
use std::time::Duration;

use jiff::Timestamp;
use jiff::tz::{TimeZone, offset};
use tokio::sync::mpsc;
use zajil_core::limit::{Clock, ManualClock};
use zajil_core::queue::{Day, QueueEvent, QueueStore, Runner, Schedule, Status, Summary};
use zajil_core::{Engine, Event, Options, Stop};
use zajil_testserver::{FileSpec, TestServer, random_bytes};

const MIB: usize = 1024 * 1024;

/// Friday 23 October 2026, 13:00:00 in Riyadh (UTC+3).
const FRIDAY_1300: &str = "2026-10-23T10:00:00Z";

fn riyadh() -> TimeZone {
    TimeZone::fixed(offset(3))
}

/// An event, and the time of the clock when the runner reported it.
type Seen = (QueueEvent, Timestamp);

struct Run {
    clock: Arc<ManualClock>,
    events: mpsc::UnboundedReceiver<Seen>,
    task: tokio::task::JoinHandle<Summary>,
}

impl Run {
    fn start(store: &QueueStore) -> Run {
        let clock = ManualClock::frozen(FRIDAY_1300.parse().unwrap());
        let engine = Engine::new(Options {
            clock: clock.clone(),
            ..Options::default()
        })
        .unwrap();
        let runner = Runner::new(engine, store.clone())
            .poll(Duration::from_millis(20))
            .time_zone(riyadh());
        let (send, events) = mpsc::unbounded_channel();
        let at = clock.clone();
        let task = tokio::spawn(async move {
            runner
                .run(&Stop::new(), &mut |event| {
                    let _ = send.send((event, at.wall()));
                })
                .await
                .unwrap()
        });
        Run {
            clock,
            events,
            task,
        }
    }

    /// The next event that `pick` accepts (others are skipped).
    async fn until<T>(&mut self, mut pick: impl FnMut(&QueueEvent, Timestamp) -> Option<T>) -> T {
        let wait = async {
            loop {
                let (event, at) = self.events.recv().await.expect("the runner ended");
                if let Some(found) = pick(&event, at) {
                    return found;
                }
            }
        };
        tokio::time::timeout(Duration::from_secs(30), wait)
            .await
            .expect("no such event within 30 s")
    }

    /// Asserts that nothing starts for a while (the runner looks every
    /// 20 ms, so 300 ms is 15 looks).
    async fn nothing_starts(&mut self) {
        let quiet = tokio::time::timeout(Duration::from_millis(300), async {
            loop {
                match self.events.recv().await {
                    Some((event @ QueueEvent::Started { .. }, _)) => return event,
                    Some(_) => {}
                    None => std::future::pending().await,
                }
            }
        })
        .await;
        assert!(quiet.is_err(), "started too early: {quiet:?}");
    }
}

fn local(at: Timestamp) -> String {
    at.to_zoned(riyadh()).strftime("%a %H:%M:%S").to_string()
}

fn schedule(start: &str, stop: &str, days: Vec<Day>) -> Schedule {
    Schedule::new(
        Some(start.parse().unwrap()),
        Some(stop.parse().unwrap()),
        days,
    )
    .unwrap()
}

fn set_up(server: &TestServer, schedule: Schedule) -> (QueueStore, tempfile::TempDir, u64) {
    let data_dir = tempfile::tempdir().unwrap();
    let store = QueueStore::new(data_dir.path());
    let url = server.url("/f.bin");
    let downloads = data_dir.path().join("downloads");
    std::fs::create_dir(&downloads).unwrap();
    let id = store
        .update(move |q| {
            q.queue_or_new("main")?.schedule = Some(schedule);
            q.add("main", &url, &downloads)
        })
        .unwrap();
    (store, data_dir, id)
}

/// M3 success criterion: a download scheduled 5 seconds ahead starts on
/// time: not at 4 seconds, and within one second of its time.
#[tokio::test]
async fn a_download_scheduled_five_seconds_ahead_starts_on_time() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(100_000, 800)));
    let (store, _dir, id) = set_up(&server, schedule("13:00:05", "14:00", vec![]));
    let mut run = Run::start(&store);
    let scheduled: Timestamp = "2026-10-23T10:00:05Z".parse().unwrap();

    let until = run
        .until(|event, _| match event {
            QueueEvent::Waiting { until, .. } => Some(until.timestamp()),
            _ => None,
        })
        .await;
    assert_eq!(until, scheduled);

    run.clock.advance(Duration::from_secs(4));
    run.nothing_starts().await;
    run.clock.advance(Duration::from_secs(1));
    let started = run
        .until(|event, at| matches!(event, QueueEvent::Started { .. }).then_some(at))
        .await;
    let late = started.duration_since(scheduled);
    assert!(
        late >= jiff::SignedDuration::ZERO && late <= jiff::SignedDuration::from_secs(1),
        "started at {}, {late:?} after {}",
        local(started),
        local(scheduled)
    );

    let summary = run.task.await.unwrap();
    assert_eq!(summary.finished, 1);
    assert_eq!(store.load().unwrap().item(id).unwrap().status, Status::Done);
}

/// The window closes in the middle of a download: it stops (recording its
/// data) and waits; the next day's window resumes it from where it was.
#[tokio::test]
async fn a_closing_window_stops_the_download_and_the_next_one_resumes_it() {
    let server = TestServer::http();
    let data = random_bytes(8 * MIB, 801);
    let spec = FileSpec::new(data.clone()).etag(Some("\"same\""));
    // Until the window closes, each response stops after 512 KiB of its
    // MiB and holds its connection: 4 MiB at most of the 8 can come, so the
    // download cannot finish before the runner sees the window closed
    // (#295: at 1 MiB/s on each of 8 connections, it finished first on
    // Windows CI and the runner ended).
    server.put("/f.bin", spec.clone().stall_after(512 * 1024));
    let (store, _dir, id) = set_up(&server, schedule("13:00:05", "13:00:10", vec![]));
    let mut run = Run::start(&store);

    run.clock.advance(Duration::from_secs(5));
    run.until(|event, _| matches!(event, QueueEvent::Started { .. }).then_some(()))
        .await;
    // Some data is on disk, then the window closes at 13:00:10.
    run.until(|event, _| match event {
        QueueEvent::Download {
            event: Event::Progress { done, .. },
            ..
        } if *done > MIB as u64 => Some(()),
        _ => None,
    })
    .await;
    run.clock.advance(Duration::from_secs(5));
    // The runner says the queue waits, and the download stops (in either
    // order: it says so as soon as it sees the window closed).
    let (mut stopped, mut until) = (false, None);
    while !stopped || until.is_none() {
        run.until(|event, _| match event {
            QueueEvent::Stopped { .. } => {
                stopped = true;
                Some(())
            }
            QueueEvent::Waiting { until: u, .. } => {
                until = Some(u.timestamp());
                Some(())
            }
            _ => None,
        })
        .await;
    }
    assert_eq!(local(until.unwrap()), "Sat 13:00:05");
    let item = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(item.status, Status::Waiting);
    assert!(item.state_file.as_ref().unwrap().exists());
    let before = server.gets("/f.bin").len();

    // The next day, the server is fast; the download resumes.
    server.put("/f.bin", spec);
    run.clock.advance(Duration::from_secs(24 * 3600 - 5 - 1));
    run.nothing_starts().await;
    run.clock.advance(Duration::from_secs(1));
    let started = run
        .until(|event, at| matches!(event, QueueEvent::Started { .. }).then_some(at))
        .await;
    assert_eq!(local(started), "Sat 13:00:05");
    let summary = run.task.await.unwrap();
    assert_eq!(summary.finished, 1);
    let done = store.load().unwrap().item(id).unwrap().clone();
    assert_eq!(std::fs::read(done.file.unwrap()).unwrap(), data);
    let after = &server.gets("/f.bin")[before..];
    assert!(after.iter().all(|r| r.range.is_some()), "{after:?}");
}

/// Days: on Friday, a queue for Saturday mornings waits until Saturday.
#[tokio::test]
async fn a_queue_waits_for_its_day() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(100_000, 802)));
    let (store, _dir, _) = set_up(&server, schedule("09:00", "10:00", vec![Day::Sat]));
    let mut run = Run::start(&store);
    let until = run
        .until(|event, _| match event {
            QueueEvent::Waiting { until, .. } => Some(until.timestamp()),
            _ => None,
        })
        .await;
    assert_eq!(local(until), "Sat 09:00:00");
    // Friday 13:00 to Saturday 08:59:59.
    run.clock.advance(Duration::from_secs(20 * 3600 - 1));
    run.nothing_starts().await;
    run.clock.advance(Duration::from_secs(1));
    let started = run
        .until(|event, at| matches!(event, QueueEvent::Started { .. }).then_some(at))
        .await;
    assert_eq!(local(started), "Sat 09:00:00");
    assert_eq!(run.task.await.unwrap().finished, 1);
}

/// A queue without a schedule runs while another waits for its window.
#[tokio::test]
async fn an_unscheduled_queue_runs_while_another_waits() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(100_000, 803)));
    let (store, dir, night) = set_up(&server, schedule("23:00", "07:00", vec![]));
    let downloads = dir.path().join("downloads");
    let url = server.url("/f.bin");
    let now = store
        .update(move |q| q.add("now", &url, &downloads))
        .unwrap();
    let mut run = Run::start(&store);
    let finished = run
        .until(|event, _| match event {
            QueueEvent::Finished { item, .. } => Some(*item),
            _ => None,
        })
        .await;
    assert_eq!(finished, now);
    assert_eq!(
        store.load().unwrap().item(night).unwrap().status,
        Status::Waiting
    );
    run.clock.advance(Duration::from_secs(10 * 3600));
    let started = run
        .until(|event, at| match event {
            QueueEvent::Started { item, .. } => Some((*item, at)),
            _ => None,
        })
        .await;
    assert_eq!(
        (started.0, local(started.1)),
        (night, "Fri 23:00:00".into())
    );
    assert_eq!(run.task.await.unwrap().finished, 2);
}

/// #125: «Start now» runs a queue outside its window at once, and its saved
/// schedule does not change; once it has nothing left, it leaves the set
/// and the schedule rules it again.
#[tokio::test]
async fn start_now_runs_a_queue_outside_its_window_and_keeps_its_schedule() {
    let server = TestServer::http();
    server.put("/f.bin", FileSpec::new(random_bytes(100_000, 801)));
    // 02:00 to 07:00, and it is 13:00: outside its window.
    let night = schedule("02:00", "07:00", vec![]);
    let (store, _dir, id) = set_up(&server, night.clone());
    let clock = ManualClock::frozen(FRIDAY_1300.parse().unwrap());
    let engine = Engine::new(Options {
        clock: clock.clone(),
        ..Options::default()
    })
    .unwrap();
    let now: zajil_core::queue::RunNow = Default::default();
    // And a queue that is not there: it leaves the set at the first look.
    now.lock()
        .unwrap()
        .extend(["main".to_owned(), "gone".to_owned()]);
    let mut started = 0;
    let summary = tokio::time::timeout(
        Duration::from_secs(30),
        Runner::new(engine, store.clone())
            .poll(Duration::from_millis(20))
            .time_zone(riyadh())
            .run_now(now.clone())
            .run(&Stop::new(), &mut |event| {
                if matches!(event, QueueEvent::Started { .. }) {
                    started += 1;
                }
                assert!(
                    !matches!(event, QueueEvent::Waiting { .. }),
                    "started now, it waits for nothing"
                );
            }),
    )
    .await
    .expect("it ran at once")
    .unwrap();
    assert_eq!((started, summary.finished), (1, 1));
    let queues = store.load().unwrap();
    assert_eq!(queues.item(id).unwrap().status, Status::Done);
    assert_eq!(queues.queue("main").unwrap().schedule, Some(night));
    assert!(now.lock().unwrap().is_empty(), "done: it left the set");
}
