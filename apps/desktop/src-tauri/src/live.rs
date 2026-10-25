//! What the running downloads are doing now, from the runner's events: the
//! bytes on disk, the speed, and the file. The queues file has the rest.

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::Mutex;
use std::time::{Duration, Instant};

use zajil_core::Event;
use zajil_core::queue::QueueEvent;

/// The page is told at most this often that something changed, except for
/// a download that ends, which is told at once.
pub const NOTIFY_EVERY: Duration = Duration::from_millis(250);
/// A download with no progress for this long shows a speed of 0.
const STALLED_AFTER: Duration = Duration::from_secs(2);

/// What to do with a change the page should hear about.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Notify {
    /// Tell it now.
    Now,
    /// Tell it after this delay (the last change is never lost: review of
    /// #45); nothing else is scheduled until then.
    Later(Duration),
    /// A later notification is already scheduled: it will carry this one.
    Scheduled,
}

#[derive(Debug, Default)]
pub struct Live {
    inner: Mutex<Inner>,
}

#[derive(Debug, Default)]
struct Inner {
    items: HashMap<u64, ItemLive>,
    problem: Option<String>,
    last_notify: Option<Instant>,
    /// A download ended since the last notification: tell at once.
    urgent: bool,
    /// A notification is scheduled for later.
    scheduled: bool,
    /// Downloads that wait for their browser's cookies, which is not open
    /// or did not answer (M5): «افتح المتصفح ليكمل».
    waiting_for_browser: std::collections::HashSet<u64>,
}

#[derive(Debug, Clone, Default)]
pub struct ItemLive {
    pub done: u64,
    pub total: Option<u64>,
    /// Bytes per second, smoothed.
    pub speed: f64,
    pub file: Option<PathBuf>,
    pub state_file: Option<PathBuf>,
    last: Option<(Instant, u64)>,
    /// When the last progress came, to show 0 for a stalled download.
    last_progress: Option<Instant>,
}

impl Live {
    fn inner(&self) -> std::sync::MutexGuard<'_, Inner> {
        self.inner.lock().unwrap_or_else(|e| e.into_inner())
    }

    pub fn record(&self, event: &QueueEvent) {
        let mut inner = self.inner();
        match event {
            QueueEvent::Started { item, .. } => {
                inner.items.insert(*item, ItemLive::default());
                inner.waiting_for_browser.remove(item);
                inner.urgent = true;
            }
            QueueEvent::Download { item, event } => {
                let live = inner.items.entry(*item).or_default();
                match event {
                    Event::Started {
                        file,
                        state_file,
                        total,
                        from,
                    } => {
                        live.file = Some(file.clone());
                        live.state_file = Some(state_file.clone());
                        live.total = *total;
                        live.done = *from;
                        live.last = Some((Instant::now(), *from));
                    }
                    Event::Progress { done, total } => {
                        let now = Instant::now();
                        live.last_progress = Some(now);
                        if let Some((at, before)) = live.last {
                            let seconds = now.duration_since(at).as_secs_f64();
                            if seconds >= 0.2 {
                                let rate = done.saturating_sub(before) as f64 / seconds;
                                // A short memory, so the number does not jump.
                                live.speed = if live.speed == 0.0 {
                                    rate
                                } else {
                                    live.speed * 0.6 + rate * 0.4
                                };
                                live.last = Some((now, *done));
                            }
                        } else {
                            live.last = Some((now, *done));
                        }
                        live.done = *done;
                        live.total = *total;
                    }
                    Event::Restarted { .. } => {
                        live.done = 0;
                        live.speed = 0.0;
                        live.last = Some((Instant::now(), 0));
                    }
                    _ => {}
                }
            }
            QueueEvent::Finished { item, .. }
            | QueueEvent::Failed { item, .. }
            | QueueEvent::Retrying { item, .. }
            | QueueEvent::Stopped { item } => {
                inner.items.remove(item);
                inner.urgent = true;
            }
            // The app decides, once it has asked the browser.
            QueueEvent::Waiting { .. }
            | QueueEvent::NeedsBrowser { .. }
            | QueueEvent::QueueDone { .. } => {}
        }
    }

    /// Marks download `id` as waiting for its browser, or not; whether
    /// that changed.
    pub fn set_waiting_for_browser(&self, id: u64, waiting: bool) -> bool {
        let mut inner = self.inner();
        if waiting {
            inner.waiting_for_browser.insert(id)
        } else {
            inner.waiting_for_browser.remove(&id)
        }
    }

    pub fn waiting_for_browser(&self, id: u64) -> bool {
        self.inner().waiting_for_browser.contains(&id)
    }

    /// When to tell the page about a change, at `now`: at once, at most
    /// every [`NOTIFY_EVERY`] (at once when a download starts or ends), and
    /// the last change of a burst after the delay.
    pub fn notify_at(&self, now: Instant) -> Notify {
        let mut inner = self.inner();
        let wait = match inner.last_notify {
            Some(at) if !inner.urgent => NOTIFY_EVERY.saturating_sub(now.duration_since(at)),
            _ => Duration::ZERO,
        };
        if wait.is_zero() {
            inner.last_notify = Some(now);
            inner.urgent = false;
            Notify::Now
        } else if inner.scheduled {
            Notify::Scheduled
        } else {
            inner.scheduled = true;
            Notify::Later(wait)
        }
    }

    /// The scheduled notification is being sent, at `now`.
    pub fn notified_later(&self, now: Instant) {
        let mut inner = self.inner();
        inner.scheduled = false;
        inner.last_notify = Some(now);
        inner.urgent = false;
    }

    pub fn item(&self, id: u64) -> Option<ItemLive> {
        self.item_at(id, Instant::now())
    }

    /// The download `id` as it is at `now`: a speed of 0 once no progress
    /// has come for [`STALLED_AFTER`] (review of #45: it stayed at its last
    /// value).
    fn item_at(&self, id: u64, now: Instant) -> Option<ItemLive> {
        let mut item = self.inner().items.get(&id).cloned()?;
        if item
            .last_progress
            .is_some_and(|at| now.duration_since(at) >= STALLED_AFTER)
        {
            item.speed = 0.0;
        }
        Some(item)
    }

    /// Forgets the running downloads: the runner stopped with an error, so
    /// none of them runs (review of #45: their speed stayed on screen).
    pub fn clear(&self) {
        self.inner().items.clear();
    }

    pub fn set_problem(&self, problem: Option<String>) {
        self.inner().problem = problem;
    }

    pub fn problem(&self) -> Option<String> {
        self.inner().problem.clone()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use zajil_core::queue::QueueEvent;

    fn progress(live: &Live, item: u64, done: u64) {
        live.record(&QueueEvent::Download {
            item,
            event: Event::Progress {
                done,
                total: Some(1000),
            },
        });
    }

    #[test]
    fn the_page_is_told_at_most_four_times_a_second_and_never_misses_the_last_change() {
        let live = Live::default();
        let t0 = Instant::now();
        assert_eq!(live.notify_at(t0), Notify::Now);
        // Within 250 ms: later, once.
        let later = live.notify_at(t0 + Duration::from_millis(100));
        assert_eq!(later, Notify::Later(Duration::from_millis(150)));
        assert_eq!(
            live.notify_at(t0 + Duration::from_millis(200)),
            Notify::Scheduled
        );
        live.notified_later(t0 + Duration::from_millis(250));
        // After the scheduled one, the next waits its turn again.
        assert_eq!(
            live.notify_at(t0 + Duration::from_millis(300)),
            Notify::Later(Duration::from_millis(200))
        );
        live.notified_later(t0 + Duration::from_millis(500));
        assert_eq!(live.notify_at(t0 + Duration::from_millis(800)), Notify::Now);
    }

    #[test]
    fn a_download_that_starts_or_ends_is_told_at_once() {
        let live = Live::default();
        let t0 = Instant::now();
        assert_eq!(live.notify_at(t0), Notify::Now);
        live.record(&QueueEvent::Started {
            queue: "main".into(),
            item: 1,
        });
        assert_eq!(live.notify_at(t0 + Duration::from_millis(10)), Notify::Now);
        live.record(&QueueEvent::Stopped { item: 1 });
        assert_eq!(live.notify_at(t0 + Duration::from_millis(20)), Notify::Now);
    }

    #[test]
    fn a_stalled_download_shows_a_speed_of_zero() {
        let live = Live::default();
        live.record(&QueueEvent::Started {
            queue: "main".into(),
            item: 1,
        });
        progress(&live, 1, 0);
        std::thread::sleep(Duration::from_millis(250));
        progress(&live, 1, 500);
        let now = Instant::now();
        assert!(live.item_at(1, now).unwrap().speed > 0.0);
        assert_eq!(live.item_at(1, now + STALLED_AFTER).unwrap().speed, 0.0);
        // Cleared when the runner stops with an error.
        live.clear();
        assert!(live.item(1).is_none());
    }
}
