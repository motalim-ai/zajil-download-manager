//! When to try a connection again (M2): growing waits with a random part,
//! and the server's own `Retry-After` (RFC 9110 §10.2.3).

use std::hash::{BuildHasher, RandomState};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::http_date;

/// The wait before the first retry is between half of this and this.
pub const FIRST_WAIT: Duration = Duration::from_millis(500);

/// No wait between retries is longer than this.
pub const LONGEST_WAIT: Duration = Duration::from_secs(30);

/// A segment whose connection failed this many times in a row, without
/// writing anything in between, is given up (the download stops and can be
/// resumed later).
pub const MAX_FAILURES: u32 = 5;

/// A server asking to wait longer than this is not waited for: the download
/// stops and can be resumed later.
pub const LONGEST_RETRY_AFTER: Duration = Duration::from_secs(10 * 60);

/// The wait after the `failures`-th failure in a row (counted from 1): it
/// doubles each time up to [`LONGEST_WAIT`], and a random part spreads the
/// connections out ("equal jitter": between half the ceiling and the
/// ceiling), so they do not all come back at the same moment.
pub fn backoff(failures: u32) -> Duration {
    let ceiling = ceiling(failures);
    let half = ceiling / 2;
    let spread = (ceiling - half).as_millis() as u64;
    half + Duration::from_millis(random_up_to(spread))
}

/// The ceiling of the wait after `failures` failures, without overflow.
fn ceiling(failures: u32) -> Duration {
    let doublings = failures.saturating_sub(1).min(31);
    FIRST_WAIT
        .checked_mul(1u32 << doublings)
        .unwrap_or(LONGEST_WAIT)
        .min(LONGEST_WAIT)
}

/// A number from 0 to `max`, from the random keys of the standard library.
/// Not for anything secret: it only spreads retries out.
fn random_up_to(max: u64) -> u64 {
    if max == 0 {
        return 0;
    }
    let value = RandomState::new().hash_one(Instant::now());
    value % (max + 1)
}

/// How long a `Retry-After` value asks to wait: delay-seconds or an HTTP
/// date (RFC 9110 §10.2.3), measured from `now`. A date in the past means
/// no wait; an unreadable value is `None`. Seconds too large to represent
/// count as an endless wait, never as an overflow.
pub fn retry_after(value: &str, now: SystemTime) -> Option<Duration> {
    let value = value.trim();
    if !value.is_empty() && value.bytes().all(|b| b.is_ascii_digit()) {
        return Some(
            value
                .parse::<u64>()
                .map_or(Duration::MAX, Duration::from_secs),
        );
    }
    let at = http_date::parse(value)?;
    let now = now.duration_since(UNIX_EPOCH).ok()?.as_secs();
    let now = i64::try_from(now).ok()?;
    Some(Duration::from_secs(
        u64::try_from(at.saturating_sub(now)).unwrap_or(0),
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn waits_grow_and_stay_within_their_bounds() {
        for failures in 1..=12 {
            let ceiling = ceiling(failures);
            for _ in 0..50 {
                let wait = backoff(failures);
                assert!(
                    wait >= ceiling / 2 && wait <= ceiling,
                    "{failures}: {wait:?}"
                );
            }
        }
        assert_eq!(ceiling(1), FIRST_WAIT);
        assert_eq!(ceiling(2), FIRST_WAIT * 2);
        assert_eq!(ceiling(3), FIRST_WAIT * 4);
        assert_eq!(ceiling(7), LONGEST_WAIT);
    }

    #[test]
    fn huge_failure_counts_do_not_overflow() {
        for failures in [0, 32, 33, 64, u32::MAX] {
            let wait = backoff(failures);
            assert!(wait <= LONGEST_WAIT, "{failures}: {wait:?}");
        }
    }

    #[test]
    fn waits_are_spread_out() {
        let waits: std::collections::HashSet<_> = (0..100).map(|_| backoff(4)).collect();
        assert!(waits.len() > 20, "only {} different waits", waits.len());
    }

    #[test]
    fn reads_retry_after_seconds_and_dates() {
        // Thu, 01 Jan 2026 00:00:00 GMT
        let now = UNIX_EPOCH + Duration::from_secs(1_767_225_600);
        assert_eq!(retry_after("2", now), Some(Duration::from_secs(2)));
        assert_eq!(retry_after(" 0 ", now), Some(Duration::ZERO));
        assert_eq!(
            retry_after("Thu, 01 Jan 2026 00:00:30 GMT", now),
            Some(Duration::from_secs(30))
        );
        // A date in the past: no wait.
        assert_eq!(
            retry_after("Wed, 31 Dec 2025 23:00:00 GMT", now),
            Some(Duration::ZERO)
        );
    }

    #[test]
    fn unreadable_or_absurd_retry_after_values_are_safe() {
        let now = SystemTime::now();
        for bad in ["", "soon", "-1", "1.5", "2 s"] {
            assert_eq!(retry_after(bad, now), None, "{bad:?}");
        }
        assert_eq!(
            retry_after("99999999999999999999999", now),
            Some(Duration::MAX)
        );
        let far = "Sun, 06 Nov 9223372036854775807 08:49:37 GMT";
        assert_eq!(retry_after(far, now), None);
        assert!(retry_after("18446744073709551615", now).unwrap() > LONGEST_RETRY_AFTER);
    }
}
