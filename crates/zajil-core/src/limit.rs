//! Speed limits (M3): one for the whole program, one for each download.
//!
//! A [`SpeedLimit`] is a token bucket shared by every connection that uses
//! it. A connection that received `n` bytes takes `n` from the bucket
//! before it reads more: the bucket may go into debt, and the connection
//! then waits until the debt is paid at the limit's rate. Connections that
//! take at the same time queue behind each other's debt, so together they
//! never go faster than the rate. Reading less makes TCP slow the server
//! down, so the limit holds on the wire too.
//!
//! Time comes from a [`Clock`]. Tests use a [`ManualClock`] (owner's
//! decision on the M3 plan): limits are measured on a clock the test owns,
//! exactly and without depending on how fast CI is. `tokio::time::pause`
//! is not used, because it would also fire the HTTP client's real timeouts
//! while it waits for the network.

use std::fmt;
use std::future::Future;
use std::pin::Pin;
use std::sync::{Arc, Mutex, MutexGuard};
use std::time::Duration;

/// The bucket holds at most this much time of the rate: a short burst
/// after an idle moment, small enough that the average stays exact.
const BURST: Duration = Duration::from_millis(100);

/// Where limits and schedules take their time from.
pub trait Clock: Send + Sync + fmt::Debug {
    /// Time since an arbitrary start; it never goes back.
    fn now(&self) -> Duration;
    /// The time of day and date, for schedules. It moves with [`Clock::now`]
    /// (the system clock may also be set by the user meanwhile).
    fn wall(&self) -> jiff::Timestamp;
    /// Resolves when [`Clock::now`] reaches `at` (at once if it has).
    fn sleep_until(&self, at: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>>;
}

/// The real clock (`tokio::time`).
#[derive(Debug, Clone)]
pub struct SystemClock {
    start: tokio::time::Instant,
}

impl Default for SystemClock {
    fn default() -> Self {
        SystemClock {
            start: tokio::time::Instant::now(),
        }
    }
}

impl Clock for SystemClock {
    fn now(&self) -> Duration {
        self.start.elapsed()
    }

    fn wall(&self) -> jiff::Timestamp {
        jiff::Timestamp::now()
    }

    fn sleep_until(&self, at: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let deadline = self
            .start
            .checked_add(at)
            .unwrap_or_else(|| tokio::time::Instant::now() + Duration::from_secs(86_400 * 365));
        Box::pin(tokio::time::sleep_until(deadline))
    }
}

/// A clock for tests, which only the test and the waits move.
///
/// - [`ManualClock::new`]: it stands still, and a wait moves it forward to
///   its end at once. So the time it shows is exactly the time that limits
///   made downloads wait, whatever the speed of the machine.
/// - [`ManualClock::frozen`]: a wait waits until the test moves the clock
///   past its end with [`ManualClock::advance`]. For schedules: the test
///   decides when it is 23:00.
#[derive(Debug)]
pub struct ManualClock {
    /// Nanoseconds since the start; waiters watch it.
    nanos: tokio::sync::watch::Sender<u64>,
    /// [`Clock::wall`] at the start.
    start: jiff::Timestamp,
    frozen: bool,
}

impl ManualClock {
    /// A clock that jumps to the end of each wait (for limits).
    pub fn new() -> Arc<Self> {
        Self::make(jiff::Timestamp::UNIX_EPOCH, false)
    }

    /// A clock at `start` that only [`ManualClock::advance`] moves (for
    /// schedules).
    pub fn frozen(start: jiff::Timestamp) -> Arc<Self> {
        Self::make(start, true)
    }

    fn make(start: jiff::Timestamp, frozen: bool) -> Arc<Self> {
        Arc::new(ManualClock {
            nanos: tokio::sync::watch::Sender::new(0),
            start,
            frozen,
        })
    }

    /// Moves the clock forward by `by`, waking the waits that end.
    pub fn advance(&self, by: Duration) {
        let by = u64::try_from(by.as_nanos()).unwrap_or(u64::MAX);
        self.nanos.send_modify(|now| *now = now.saturating_add(by));
    }
}

impl Clock for ManualClock {
    fn now(&self) -> Duration {
        Duration::from_nanos(*self.nanos.borrow())
    }

    fn wall(&self) -> jiff::Timestamp {
        let since = jiff::SignedDuration::try_from(self.now()).unwrap_or(jiff::SignedDuration::MAX);
        self.start
            .checked_add(since)
            .unwrap_or(jiff::Timestamp::MAX)
    }

    fn sleep_until(&self, at: Duration) -> Pin<Box<dyn Future<Output = ()> + Send + '_>> {
        let at = u64::try_from(at.as_nanos()).unwrap_or(u64::MAX);
        if !self.frozen {
            self.nanos.send_modify(|now| *now = (*now).max(at));
            return Box::pin(std::future::ready(()));
        }
        let mut now = self.nanos.subscribe();
        Box::pin(async move {
            // The sender lives in `self`, so this only ends at `at`.
            let _ = now.wait_for(|&now| now >= at).await;
        })
    }
}

/// A speed limit in bytes per second, shared by every connection that
/// takes from it. Cheap to clone: clones share the same bucket. Its rate
/// can change while downloads use it; `None` means no limit.
#[derive(Clone)]
pub struct SpeedLimit {
    inner: Arc<Bucket>,
}

struct Bucket {
    clock: Arc<dyn Clock>,
    state: Mutex<State>,
    /// Counts rate changes, so that waiting connections wake up and wait
    /// by the new rate (review of #32: removing a limit did not reach
    /// them for minutes).
    changes: tokio::sync::watch::Sender<u64>,
}

struct State {
    rate: Option<u64>,
    /// Bytes that may still pass without waiting; below zero, a debt.
    tokens: f64,
    /// When `tokens` was last brought up to date.
    at: Duration,
}

impl fmt::Debug for SpeedLimit {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("SpeedLimit")
            .field("rate", &self.rate())
            .finish()
    }
}

impl SpeedLimit {
    /// A limit of `rate` bytes per second (`None`: no limit) on `clock`.
    pub fn new(rate: Option<u64>, clock: Arc<dyn Clock>) -> Self {
        let at = clock.now();
        SpeedLimit {
            inner: Arc::new(Bucket {
                clock,
                state: Mutex::new(State {
                    rate: rate.filter(|&r| r > 0),
                    tokens: 0.0,
                    at,
                }),
                changes: tokio::sync::watch::Sender::new(0),
            }),
        }
    }

    pub fn rate(&self) -> Option<u64> {
        self.state().rate
    }

    /// Changes the rate for what comes next (`None`: no limit). A debt
    /// already owed is forgiven when the limit is removed.
    /// Connections waiting for this limit wake up and wait by the new rate
    /// (not at all without one).
    pub fn set_rate(&self, rate: Option<u64>) {
        let rate = rate.filter(|&r| r > 0);
        {
            let mut state = self.state();
            if state.rate == rate {
                return;
            }
            let now = self.now(&state);
            refill(&mut state, now);
            state.rate = rate;
            if state.rate.is_none() {
                state.tokens = 0.0;
            }
        }
        self.inner.changes.send_modify(|count| *count += 1);
    }

    /// Takes `bytes` that a connection received, and waits as long as the
    /// limit asks before it may read more.
    pub async fn take(&self, bytes: u64) {
        let changes = self.inner.changes.subscribe();
        if let Some(until) = self.reserve(bytes) {
            self.wait(until, changes).await;
        }
    }

    /// Gives back bytes taken but then dropped, not written (a connection
    /// read past the end of its part): the limit counts what is saved.
    ///
    /// The wait for them is usually over already, so they become credit
    /// that the next reads use without waiting, even above the burst: the
    /// average of what is saved stays at the rate.
    pub fn give_back(&self, bytes: u64) {
        let mut state = self.state();
        if state.rate.is_some() {
            let now = self.now(&state);
            refill(&mut state, now);
            state.tokens += bytes as f64;
        }
    }

    /// Waits until `until`, or, when the rate changes meanwhile, until the
    /// debt is paid at the new rate.
    async fn wait(&self, mut until: Duration, mut changes: tokio::sync::watch::Receiver<u64>) {
        loop {
            tokio::select! {
                () = self.inner.clock.sleep_until(until) => return,
                changed = changes.changed() => {
                    if changed.is_err() {
                        return;
                    }
                    match self.deadline() {
                        Some(next) => until = next,
                        None => return,
                    }
                }
            }
        }
    }

    /// When the current debt is paid, if there is one.
    fn deadline(&self) -> Option<Duration> {
        let mut state = self.state();
        let rate = state.rate?;
        let now = self.now(&state);
        refill(&mut state, now);
        (state.tokens < 0.0).then(|| {
            now.saturating_add(Duration::from_secs_f64(
                (-state.tokens / rate as f64).min(86_400.0),
            ))
        })
    }

    /// Takes `bytes` without waiting, and returns when the wait it asks for
    /// ends (on this limit's clock), if there is one.
    fn reserve(&self, bytes: u64) -> Option<Duration> {
        let mut state = self.state();
        let rate = state.rate?;
        let now = self.now(&state);
        refill(&mut state, now);
        state.tokens -= bytes as f64;
        (state.tokens < 0.0).then(|| {
            now.saturating_add(Duration::from_secs_f64(
                (-state.tokens / rate as f64).min(86_400.0),
            ))
        })
    }

    /// The time, read while the bucket is locked, and never before the
    /// bucket's last update: a time read before another connection moved
    /// on would count the same moment twice.
    fn now(&self, state: &State) -> Duration {
        self.inner.clock.now().max(state.at)
    }

    fn state(&self) -> MutexGuard<'_, State> {
        self.inner.state.lock().unwrap_or_else(|e| e.into_inner())
    }
}

/// Adds what the rate gave since the last update, up to [`BURST`]. Credit
/// above it (bytes given back) is kept, not cut.
fn refill(state: &mut State, now: Duration) {
    let elapsed = now.saturating_sub(state.at);
    state.at = now;
    if let Some(rate) = state.rate {
        let most = rate as f64 * BURST.as_secs_f64();
        let refilled = (state.tokens + elapsed.as_secs_f64() * rate as f64).min(most);
        state.tokens = state.tokens.max(refilled);
    }
}

/// The limits one download goes through: the program's and its own.
#[derive(Debug, Clone, Default)]
pub(crate) struct Limits {
    pub limits: Vec<SpeedLimit>,
}

impl Limits {
    /// Takes `bytes` from every limit at once, then waits until the last of
    /// their waits ends. Waiting for one limit after the other would add
    /// the waits up instead (and the second limit would lose what it saved
    /// meanwhile, above its burst): two limits would then be slower than
    /// the lower of them. Found by `the_runner_applies_the_limits_of_the_queues_file`.
    pub async fn take(&self, bytes: u64) {
        let waits: Vec<_> = self
            .limits
            .iter()
            .filter_map(|limit| {
                let changes = limit.inner.changes.subscribe();
                Some((limit, limit.reserve(bytes)?, changes))
            })
            .collect();
        // The waits are moments, not lengths: on one clock, sleeping until
        // each in turn ends at the latest.
        for (limit, until, changes) in waits {
            limit.wait(until, changes).await;
        }
    }

    /// Whether any of the limits has a rate now.
    pub fn active(&self) -> bool {
        self.limits.iter().any(|limit| limit.rate().is_some())
    }

    /// See [`SpeedLimit::give_back`].
    pub fn give_back(&self, bytes: u64) {
        for limit in &self.limits {
            limit.give_back(bytes);
        }
    }
}

/// Reads a rate such as `1M`, `500K`, `1.5MiB/s` or `2048` (bytes per
/// second). `K`, `M` and `G` are powers of 1024 (KiB, MiB, GiB), like the
/// sizes Zajil shows. `None` for anything else, zero included.
pub fn parse_rate(text: &str) -> Option<u64> {
    let text = text.trim();
    let text = text.strip_suffix("/s").unwrap_or(text).trim_end();
    let text = text
        .strip_suffix("iB")
        .or_else(|| text.strip_suffix('B'))
        .unwrap_or(text);
    let (number, unit) = match text.char_indices().last() {
        Some((i, c)) if c.is_ascii_alphabetic() => (&text[..i], Some(c)),
        _ => (text, None),
    };
    let factor: u64 = match unit.map(|c| c.to_ascii_uppercase()) {
        None => 1,
        Some('K') => 1 << 10,
        Some('M') => 1 << 20,
        Some('G') => 1 << 30,
        Some(_) => return None,
    };
    let number = number.trim();
    if number.is_empty() || !number.bytes().all(|b| b.is_ascii_digit() || b == b'.') {
        return None;
    }
    let value: f64 = number.parse().ok()?;
    let bytes = value * factor as f64;
    (bytes >= 1.0 && bytes < u64::MAX as f64).then_some(bytes as u64)
}

#[cfg(test)]
mod tests {
    use super::*;

    const MIB: u64 = 1 << 20;

    /// Pulls `total` bytes through `limit` in `chunk`-sized pieces as fast
    /// as it can, like a connection on a fast server.
    async fn pull(limit: &SpeedLimit, total: u64, chunk: u64) {
        let mut left = total;
        while left > 0 {
            let n = chunk.min(left);
            limit.take(n).await;
            left -= n;
        }
    }

    #[tokio::test]
    async fn one_reader_gets_the_rate() {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(Some(MIB), clock.clone());
        pull(&limit, 6 * MIB, 64 * 1024).await;
        let speed = 6.0 / clock.now().as_secs_f64();
        assert!((0.99..=1.01).contains(&speed), "{speed} MiB/s");
    }

    #[tokio::test]
    async fn readers_sharing_a_limit_get_the_rate_together() {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(Some(MIB), clock.clone());
        let readers: Vec<_> = (0..8)
            .map(|_| {
                let limit = limit.clone();
                tokio::spawn(async move { pull(&limit, MIB, 16 * 1024).await })
            })
            .collect();
        for reader in readers {
            reader.await.unwrap();
        }
        let speed = 8.0 / clock.now().as_secs_f64();
        assert!((0.99..=1.01).contains(&speed), "{speed} MiB/s");
    }

    #[tokio::test]
    async fn no_limit_never_waits_and_the_rate_can_change() {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(None, clock.clone());
        pull(&limit, 100 * MIB, MIB).await;
        assert_eq!(clock.now(), Duration::ZERO);

        limit.set_rate(Some(2 * MIB));
        pull(&limit, 4 * MIB, 64 * 1024).await;
        let first = clock.now();
        assert!((1.9..=2.1).contains(&first.as_secs_f64()), "{first:?}");
        limit.set_rate(Some(MIB / 2));
        pull(&limit, MIB, 64 * 1024).await;
        let second = clock.now() - first;
        assert!((1.9..=2.1).contains(&second.as_secs_f64()), "{second:?}");
        assert_eq!(limit.rate(), Some(MIB / 2));
    }

    #[tokio::test]
    async fn an_idle_limit_allows_only_a_short_burst() {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(Some(MIB), clock.clone());
        clock.advance(Duration::from_secs(60));
        let before = clock.now();
        pull(&limit, 3 * MIB, 64 * 1024).await;
        let took = (clock.now() - before).as_secs_f64();
        // A full minute idle saves only 0.1 s of the rate.
        assert!((2.85..=2.95).contains(&took), "{took}");
    }

    /// Review of #32: removing or raising a limit did not reach the
    /// connections already waiting (256 s for one waiting 256 KiB at
    /// 1 KiB/s). Real clock: without the fix these waits take 256 s and
    /// 64 s; the test allows 5 s each.
    #[tokio::test]
    async fn a_new_rate_reaches_connections_already_waiting() {
        let clock: Arc<dyn Clock> = Arc::new(SystemClock::default());
        let limit = SpeedLimit::new(Some(1024), clock);
        let waiting = tokio::spawn({
            let limit = limit.clone();
            async move { limit.take(256 * 1024).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        assert!(!waiting.is_finished());
        limit.set_rate(None);
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("still waiting after the limit was removed")
            .unwrap();

        // Raised: the debt of 64 KiB takes 1 s at 64 KiB/s, not 64 s.
        limit.set_rate(Some(1024));
        let waiting = tokio::spawn({
            let limit = limit.clone();
            async move { limit.take(64 * 1024).await }
        });
        tokio::time::sleep(Duration::from_millis(50)).await;
        limit.set_rate(Some(64 * 1024));
        tokio::time::timeout(Duration::from_secs(5), waiting)
            .await
            .expect("still waiting long after the new deadline")
            .unwrap();
    }

    #[tokio::test]
    async fn bytes_given_back_are_not_counted() {
        let clock = ManualClock::new();
        let limit = SpeedLimit::new(Some(MIB), clock.clone());
        // 1 MiB taken and waited for, half of it dropped and given back:
        // the next half MiB passes without waiting.
        limit.take(MIB).await;
        let waited = clock.now();
        limit.give_back(MIB / 2);
        limit.take(MIB / 2).await;
        assert_eq!(clock.now(), waited);
        // And the credit is spent: the next byte waits again.
        limit.take(MIB / 4).await;
        assert!(clock.now() > waited);
    }

    #[tokio::test]
    async fn two_limits_cost_the_lower_one_not_their_sum() {
        let clock = ManualClock::new();
        let limits = Limits {
            limits: vec![
                SpeedLimit::new(Some(MIB), clock.clone()),
                SpeedLimit::new(Some(MIB / 2), clock.clone()),
            ],
        };
        let mut left = 4 * MIB;
        while left > 0 {
            limits.take(256 * 1024).await;
            left -= 256 * 1024;
        }
        let took = clock.now().as_secs_f64();
        assert!((7.8..=8.1).contains(&took), "{took}");
    }

    #[tokio::test]
    async fn the_real_clock_sleeps() {
        let clock = Arc::new(SystemClock::default());
        let before = clock.now();
        clock.sleep_until(before + Duration::from_millis(20)).await;
        assert!(clock.now() >= before + Duration::from_millis(20));
    }

    #[test]
    fn reads_rates() {
        for (text, rate) in [
            ("1M", MIB),
            ("1m", MIB),
            ("1MiB/s", MIB),
            ("1MB", MIB),
            ("500K", 500 * 1024),
            ("1.5M", 3 * MIB / 2),
            ("2048", 2048),
            ("1G", 1 << 30),
        ] {
            assert_eq!(parse_rate(text), Some(rate), "{text}");
        }
        for bad in [
            "",
            "0",
            "-1M",
            "1X",
            "M",
            "1e9",
            "fast",
            "0.1",
            "99999999999999999999G",
        ] {
            assert_eq!(parse_rate(bad), None, "{bad}");
        }
    }
}
