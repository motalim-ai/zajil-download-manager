//! When a queue may run (M3): a window each day, from `start` to `stop` in
//! local time, on the chosen days.
//!
//! - A window begins at `start` (00:00 without one) on each chosen day
//!   (every day without any).
//! - It ends at `stop` the same day, or the next day if `stop` is not after
//!   `start` (`23:00` to `07:00`). Without `stop` it ends at midnight.
//! - Outside its windows a queue starts nothing, and its running downloads
//!   stop (and wait) and go back to waiting; they resume at the next
//!   window.
//!
//! Local time follows the system's time zone, with its daylight saving
//! rules (`jiff`). A start that falls in a skipped hour (spring forward)
//! happens just after it; one in a repeated hour (fall back) happens the
//! first time.

use std::fmt;
use std::str::FromStr;

use jiff::Timestamp;
use jiff::civil::{Date, Time, Weekday};
use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A queue's windows.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Schedule {
    /// When a window begins; `None`: at 00:00.
    pub start: Option<TimeOfDay>,
    /// When it ends; `None`: at midnight.
    pub stop: Option<TimeOfDay>,
    /// The days a window begins on; empty: every day.
    #[serde(default)]
    pub days: Vec<Day>,
}

/// A local time of day, `HH:MM` or `HH:MM:SS` (24-hour).
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct TimeOfDay(Time);

/// A day of the week, written `mon` to `sun`.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Day {
    Mon,
    Tue,
    Wed,
    Thu,
    Fri,
    Sat,
    Sun,
}

/// A time of day or a day that could not be read.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ScheduleError {
    #[error("{0:?} is not a time of day: use HH:MM or HH:MM:SS, such as 23:00")]
    Time(String),
    #[error("{0:?} is not a day: use mon, tue, wed, thu, fri, sat or sun")]
    Day(String),
    #[error("a schedule needs a start, a stop or days")]
    Empty,
}

impl Schedule {
    pub fn new(
        start: Option<TimeOfDay>,
        stop: Option<TimeOfDay>,
        mut days: Vec<Day>,
    ) -> Result<Self, ScheduleError> {
        if start.is_none() && stop.is_none() && days.is_empty() {
            return Err(ScheduleError::Empty);
        }
        days.sort_by_key(|d| d.weekday().to_monday_zero_offset());
        days.dedup();
        Ok(Schedule { start, stop, days })
    }

    /// Whether the queue may run at `now`.
    pub fn is_open(&self, now: Timestamp, tz: &TimeZone) -> bool {
        self.window_around(now, tz).is_some()
    }

    /// When [`Schedule::is_open`] next changes: the end of the window
    /// `now` is in, or the start of the next one.
    pub fn next_change(&self, now: Timestamp, tz: &TimeZone) -> Timestamp {
        if let Some((_, end)) = self.window_around(now, tz) {
            return end;
        }
        let today = now.to_zoned(tz.clone()).date();
        // Every day of the next week, and one more for a window that began
        // yesterday.
        (0..=8)
            .filter_map(|k| today.checked_add(jiff::Span::new().days(k)).ok())
            .filter_map(|day| self.window(day, tz))
            .map(|(start, _)| start)
            .find(|&start| start > now)
            .unwrap_or_else(|| {
                now.saturating_add(jiff::SignedDuration::from_hours(24))
                    .unwrap_or(now)
            })
    }

    /// The window `now` is in, if any: one that began yesterday (and
    /// crosses midnight) or today.
    fn window_around(&self, now: Timestamp, tz: &TimeZone) -> Option<(Timestamp, Timestamp)> {
        let today = now.to_zoned(tz.clone()).date();
        let yesterday = today.yesterday().ok();
        [yesterday, Some(today)]
            .into_iter()
            .flatten()
            .filter_map(|day| self.window(day, tz))
            .find(|&(start, end)| start <= now && now < end)
    }

    /// The window that begins on `day`, if it is one of the days.
    fn window(&self, day: Date, tz: &TimeZone) -> Option<(Timestamp, Timestamp)> {
        if !self.days.is_empty() && !self.days.iter().any(|d| d.weekday() == day.weekday()) {
            return None;
        }
        let start = self.start.map_or(Time::midnight(), |t| t.0);
        let end_day = match self.stop {
            Some(stop) if stop.0 > start => day,
            _ => day.tomorrow().ok()?,
        };
        let stop = self.stop.map_or(Time::midnight(), |t| t.0);
        let at = |date: Date, time: Time| {
            date.to_datetime(time)
                .to_zoned(tz.clone())
                .ok()
                .map(|z| z.timestamp())
        };
        Some((at(day, start)?, at(end_day, stop)?))
    }
}

impl fmt::Display for Schedule {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let start = self.start.map_or("00:00".to_owned(), |t| t.to_string());
        let stop = self.stop.map_or("24:00".to_owned(), |t| t.to_string());
        write!(f, "{start} to {stop}")?;
        if self.days.is_empty() {
            write!(f, ", every day")
        } else {
            let days: Vec<String> = self.days.iter().map(|d| d.to_string()).collect();
            write!(f, ", on {}", days.join(","))
        }
    }
}

impl TimeOfDay {
    pub fn new(hour: i8, minute: i8, second: i8) -> Option<Self> {
        Time::new(hour, minute, second, 0).ok().map(TimeOfDay)
    }
}

impl FromStr for TimeOfDay {
    type Err = ScheduleError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let bad = || ScheduleError::Time(text.to_owned());
        let parts: Vec<&str> = text.split(':').collect();
        if !(2..=3).contains(&parts.len())
            || parts
                .iter()
                .any(|p| p.len() != 2 || !p.bytes().all(|b| b.is_ascii_digit()))
        {
            return Err(bad());
        }
        let number = |i: usize| {
            parts
                .get(i)
                .map_or(Ok(0), |p| p.parse::<i8>().map_err(|_| bad()))
        };
        TimeOfDay::new(number(0)?, number(1)?, number(2)?).ok_or_else(bad)
    }
}

impl fmt::Display for TimeOfDay {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let t = self.0;
        if t.second() == 0 {
            write!(f, "{:02}:{:02}", t.hour(), t.minute())
        } else {
            write!(f, "{:02}:{:02}:{:02}", t.hour(), t.minute(), t.second())
        }
    }
}

impl Serialize for TimeOfDay {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(self)
    }
}

impl<'de> Deserialize<'de> for TimeOfDay {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let text = String::deserialize(deserializer)?;
        text.parse().map_err(serde::de::Error::custom)
    }
}

impl Day {
    fn weekday(self) -> Weekday {
        match self {
            Day::Mon => Weekday::Monday,
            Day::Tue => Weekday::Tuesday,
            Day::Wed => Weekday::Wednesday,
            Day::Thu => Weekday::Thursday,
            Day::Fri => Weekday::Friday,
            Day::Sat => Weekday::Saturday,
            Day::Sun => Weekday::Sunday,
        }
    }

    const ALL: [(Day, &'static str); 7] = [
        (Day::Mon, "mon"),
        (Day::Tue, "tue"),
        (Day::Wed, "wed"),
        (Day::Thu, "thu"),
        (Day::Fri, "fri"),
        (Day::Sat, "sat"),
        (Day::Sun, "sun"),
    ];
}

impl FromStr for Day {
    type Err = ScheduleError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        Day::ALL
            .iter()
            .find(|(_, name)| name.eq_ignore_ascii_case(text.trim()))
            .map(|(day, _)| *day)
            .ok_or_else(|| ScheduleError::Day(text.to_owned()))
    }
}

impl fmt::Display for Day {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        let name = Day::ALL
            .iter()
            .find(|(d, _)| d == self)
            .map_or("?", |(_, n)| n);
        f.write_str(name)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn at(text: &str, tz: &TimeZone) -> Timestamp {
        let dt: jiff::civil::DateTime = text.parse().unwrap();
        dt.to_zoned(tz.clone()).unwrap().timestamp()
    }

    fn time(text: &str) -> Option<TimeOfDay> {
        Some(text.parse().unwrap())
    }

    // Friday 23 October 2026, in a zone 3 hours ahead of UTC.
    fn riyadh() -> TimeZone {
        TimeZone::fixed(jiff::tz::offset(3))
    }

    #[test]
    fn a_whole_minute_is_shown_without_seconds() {
        // What `zajil queue list` prints, and what its real-clock test
        // compares with (the test failed when a start fell on :00).
        assert_eq!(
            "22:51:00".parse::<TimeOfDay>().unwrap().to_string(),
            "22:51"
        );
        assert_eq!(
            "22:51:07".parse::<TimeOfDay>().unwrap().to_string(),
            "22:51:07"
        );
    }

    #[test]
    fn a_window_within_a_day() {
        let tz = riyadh();
        let s = Schedule::new(time("09:00"), time("17:30"), vec![]).unwrap();
        assert!(!s.is_open(at("2026-10-23T08:59:59", &tz), &tz));
        assert!(s.is_open(at("2026-10-23T09:00:00", &tz), &tz));
        assert!(s.is_open(at("2026-10-23T17:29:59", &tz), &tz));
        assert!(!s.is_open(at("2026-10-23T17:30:00", &tz), &tz));
        assert_eq!(
            s.next_change(at("2026-10-23T12:00", &tz), &tz),
            at("2026-10-23T17:30", &tz)
        );
        assert_eq!(
            s.next_change(at("2026-10-23T18:00", &tz), &tz),
            at("2026-10-24T09:00", &tz)
        );
    }

    #[test]
    fn a_night_window_crosses_midnight() {
        let tz = riyadh();
        let s = Schedule::new(time("23:00"), time("07:00"), vec![]).unwrap();
        assert!(s.is_open(at("2026-10-23T23:30", &tz), &tz));
        assert!(s.is_open(at("2026-10-24T06:59:59", &tz), &tz));
        assert!(!s.is_open(at("2026-10-24T07:00", &tz), &tz));
        assert_eq!(
            s.next_change(at("2026-10-24T02:00", &tz), &tz),
            at("2026-10-24T07:00", &tz)
        );
    }

    #[test]
    fn days_choose_where_windows_begin() {
        let tz = riyadh();
        // Friday 23 October 2026. A night window on Saturday only.
        let s = Schedule::new(time("23:00"), time("07:00"), vec![Day::Sat]).unwrap();
        assert!(!s.is_open(at("2026-10-23T23:30", &tz), &tz), "Friday");
        assert_eq!(
            s.next_change(at("2026-10-23T23:30", &tz), &tz),
            at("2026-10-24T23:00", &tz)
        );
        // It began on Saturday, so it still runs early on Sunday.
        assert!(s.is_open(at("2026-10-25T03:00", &tz), &tz));
        assert!(!s.is_open(at("2026-10-26T03:00", &tz), &tz));
        // A whole day: only days.
        let weekend = Schedule::new(None, None, vec![Day::Fri, Day::Sat]).unwrap();
        assert!(weekend.is_open(at("2026-10-23T00:00", &tz), &tz));
        assert!(weekend.is_open(at("2026-10-24T23:59:59", &tz), &tz));
        assert!(!weekend.is_open(at("2026-10-25T00:00", &tz), &tz));
        assert_eq!(
            weekend.next_change(at("2026-10-25T10:00", &tz), &tz),
            at("2026-10-30T00:00", &tz)
        );
    }

    #[test]
    fn a_start_alone_runs_until_midnight_and_a_stop_alone_from_midnight() {
        let tz = riyadh();
        let start = Schedule::new(time("22:00"), None, vec![]).unwrap();
        assert!(start.is_open(at("2026-10-23T23:59:59", &tz), &tz));
        assert!(!start.is_open(at("2026-10-24T00:00", &tz), &tz));
        let stop = Schedule::new(None, time("06:00"), vec![]).unwrap();
        assert!(stop.is_open(at("2026-10-24T05:00", &tz), &tz));
        assert!(!stop.is_open(at("2026-10-24T06:00", &tz), &tz));
    }

    #[test]
    fn daylight_saving_changes_follow_the_local_clock() {
        // New York: 8 March 2026 at 02:00 the clock jumps to 03:00; on 1
        // November at 02:00 it goes back to 01:00.
        let tz = TimeZone::posix("EST5EDT,M3.2.0,M11.1.0").unwrap();
        let s = Schedule::new(time("02:30"), time("04:00"), vec![]).unwrap();
        // 02:30 does not exist that day: the window opens at 03:30 EDT.
        let opens = s.next_change(at("2026-03-08T01:00", &tz), &tz);
        assert_eq!(
            opens.to_zoned(tz.clone()).time(),
            Time::constant(3, 30, 0, 0)
        );
        // A window of 01:00 to 03:00 on the night the clock goes back
        // lasts three hours.
        let s = Schedule::new(time("01:00"), time("03:00"), vec![]).unwrap();
        let start = s.next_change(at("2026-11-01T00:00", &tz), &tz);
        let end = s.next_change(start, &tz);
        assert_eq!(
            end.duration_since(start),
            jiff::SignedDuration::from_hours(3)
        );
    }

    #[test]
    fn times_and_days_are_read_strictly() {
        assert_eq!(time("07:05"), TimeOfDay::new(7, 5, 0));
        assert_eq!(time("23:59:59"), TimeOfDay::new(23, 59, 59));
        for bad in [
            "24:00",
            "7:05",
            "07:5",
            "07",
            "07:05:60",
            "-1:00",
            "aa:bb",
            "07:05:00:00",
            "",
        ] {
            assert!(bad.parse::<TimeOfDay>().is_err(), "{bad}");
        }
        assert_eq!("Mon".parse::<Day>(), Ok(Day::Mon));
        assert!("monday".parse::<Day>().is_err());
        assert_eq!(Schedule::new(None, None, vec![]), Err(ScheduleError::Empty));
        let s = Schedule::new(
            time("23:00"),
            time("07:00:30"),
            vec![Day::Sat, Day::Mon, Day::Sat],
        )
        .unwrap();
        assert_eq!(s.to_string(), "23:00 to 07:00:30, on mon,sat");
        let json = serde_json::to_string(&s).unwrap();
        assert_eq!(
            json,
            r#"{"start":"23:00","stop":"07:00:30","days":["mon","sat"]}"#
        );
        assert_eq!(serde_json::from_str::<Schedule>(&json).unwrap(), s);
        assert!(serde_json::from_str::<Schedule>(r#"{"start":"25:00","stop":null}"#).is_err());
    }
}
