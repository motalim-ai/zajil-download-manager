//! HTTP dates (RFC 9110 §5.6.7).
//!
//! Recipients must accept three formats: IMF-fixdate (the current one), the
//! obsolete RFC 850 format and asctime. All are in UTC.

/// The years we accept. A date outside them comes from a broken or hostile
/// server (or a crafted state file), and is treated as absent rather than
/// compared: it can never make a validator look strong. The range also
/// keeps every computation far from integer overflow.
const YEARS: std::ops::RangeInclusive<i64> = 1900..=9999;

/// Seconds since 1970-01-01T00:00:00Z, or `None` if `value` is not an HTTP
/// date in [`YEARS`]. Never panics, in debug or release builds.
pub fn parse(value: &str) -> Option<i64> {
    let value = value.trim();
    imf_fixdate(value)
        .or_else(|| rfc850(value))
        .or_else(|| asctime(value))
}

/// `Sun, 06 Nov 1994 08:49:37 GMT`
fn imf_fixdate(value: &str) -> Option<i64> {
    let (_weekday, rest) = value.split_once(", ")?;
    let parts: Vec<&str> = rest.split(' ').collect();
    let [day, month, year, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    build(year.parse().ok()?, month, day, time)
}

/// `Sunday, 06-Nov-94 08:49:37 GMT`
fn rfc850(value: &str) -> Option<i64> {
    let (_weekday, rest) = value.split_once(", ")?;
    let parts: Vec<&str> = rest.split(' ').collect();
    let [date, time, "GMT"] = parts.as_slice() else {
        return None;
    };
    let mut fields = date.split('-');
    let (day, month, year) = (fields.next()?, fields.next()?, fields.next()?);
    if fields.next().is_some() || year.len() != 2 {
        return None;
    }
    let yy: i64 = year.parse().ok()?;
    // Two-digit years: 70–99 are 19xx, the rest 20xx.
    let year = if yy >= 70 { 1900 + yy } else { 2000 + yy };
    build(year, month, day, time)
}

/// `Sun Nov  6 08:49:37 1994`
fn asctime(value: &str) -> Option<i64> {
    let parts: Vec<&str> = value.split_whitespace().collect();
    let [_weekday, month, day, time, year] = parts.as_slice() else {
        return None;
    };
    build(year.parse().ok()?, month, day, time)
}

fn build(year: i64, month: &str, day: &str, time: &str) -> Option<i64> {
    if !YEARS.contains(&year) {
        return None;
    }
    const MONTHS: [&str; 12] = [
        "Jan", "Feb", "Mar", "Apr", "May", "Jun", "Jul", "Aug", "Sep", "Oct", "Nov", "Dec",
    ];
    let month = MONTHS.iter().position(|m| *m == month)? as i64 + 1;
    let day: i64 = day.parse().ok()?;
    let mut hms = time.split(':');
    let (h, m, s): (i64, i64, i64) = (
        hms.next()?.parse().ok()?,
        hms.next()?.parse().ok()?,
        hms.next()?.parse().ok()?,
    );
    let valid = (1..=days_in_month(year, month)).contains(&day)
        && (0..24).contains(&h)
        && (0..60).contains(&m)
        && (0..=60).contains(&s)
        && hms.next().is_none();
    valid.then(|| days_from_civil(year, month, day) * 86_400 + h * 3600 + m * 60 + s)
}

/// 28 to 31: days that really exist in `month` of `year` (Gregorian), so a
/// date such as 31 February is refused rather than read as 3 March.
fn days_in_month(year: i64, month: i64) -> i64 {
    match month {
        4 | 6 | 9 | 11 => 30,
        2 if year % 4 == 0 && (year % 100 != 0 || year % 400 == 0) => 29,
        2 => 28,
        _ => 31,
    }
}

/// Days since 1970-01-01 for a proleptic Gregorian date (Howard Hinnant's
/// public-domain `days_from_civil` algorithm).
fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let yoe = year - era * 400;
    let mp = (month + 9) % 12;
    let doy = (153 * mp + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    era * 146_097 + doe - 719_468
}

#[cfg(test)]
mod tests {
    use super::*;

    const EXAMPLE: i64 = 784_111_777; // 1994-11-06T08:49:37Z, RFC 9110's example

    #[test]
    fn parses_the_three_formats_of_rfc_9110() {
        assert_eq!(parse("Sun, 06 Nov 1994 08:49:37 GMT"), Some(EXAMPLE));
        assert_eq!(parse("Sunday, 06-Nov-94 08:49:37 GMT"), Some(EXAMPLE));
        assert_eq!(parse("Sun Nov  6 08:49:37 1994"), Some(EXAMPLE));
    }

    #[test]
    fn parses_recent_dates() {
        assert_eq!(parse("Thu, 01 Jan 1970 00:00:00 GMT"), Some(0));
        assert_eq!(
            parse("Wed, 21 Oct 2026 07:28:00 GMT").unwrap()
                - parse("Wed, 21 Oct 2026 07:27:59 GMT").unwrap(),
            1
        );
        assert_eq!(parse("Thu, 29 Feb 2024 00:00:00 GMT"), Some(1_709_164_800));
    }

    #[test]
    fn treats_unreasonable_years_as_absent_without_panicking() {
        // Review of #17: this year overflowed (`attempt to multiply with
        // overflow` in debug builds, a meaningless value in release).
        for bad in [
            "Sun, 06 Nov 9223372036854775807 08:49:37 GMT",
            "Sun, 06 Nov -9223372036854775808 08:49:37 GMT",
            "Sun, 06 Nov 99999999999999999999999 08:49:37 GMT",
            "Sun Nov  6 08:49:37 9223372036854775807",
            "Sun, 06 Nov 1899 23:59:59 GMT",
            "Sun, 06 Nov 10000 00:00:00 GMT",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        assert!(parse("Mon, 01 Jan 1900 00:00:00 GMT").is_some());
        assert!(parse("Fri, 31 Dec 9999 23:59:59 GMT").is_some());
    }

    #[test]
    fn rejects_days_that_do_not_exist() {
        // Final review: 31 February was accepted and read as 3 March.
        for bad in [
            "Tue, 31 Feb 2026 08:49:37 GMT",
            "Mon, 29 Feb 2027 00:00:00 GMT",
            "Tue, 29 Feb 2100 00:00:00 GMT", // not a leap year
            "Fri, 31 Apr 2026 00:00:00 GMT",
            "Mon, 31 Jun 2026 00:00:00 GMT",
            "Thu, 31 Sep 2026 00:00:00 GMT",
            "Tue, 31 Nov 2026 00:00:00 GMT",
            "Wed, 00 Oct 2026 00:00:00 GMT",
            "Tuesday, 31-Feb-26 08:49:37 GMT",
            "Tue Feb 31 08:49:37 2026",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
        for good in [
            "Thu, 29 Feb 2024 00:00:00 GMT",
            "Tue, 29 Feb 2000 00:00:00 GMT", // divisible by 400
            "Sat, 31 Jan 2026 00:00:00 GMT",
            "Thu, 30 Apr 2026 00:00:00 GMT",
            "Thu, 31 Dec 2026 23:59:59 GMT",
        ] {
            assert!(parse(good).is_some(), "{good:?}");
        }
    }

    #[test]
    fn rejects_anything_else() {
        for bad in [
            "",
            "yesterday",
            "Sun, 06 Nov 1994 08:49:37 UTC",
            "Sun, 06 Nov 1994 08:49 GMT",
            "Sun, 32 Nov 1994 08:49:37 GMT",
            "Sun, 06 Foo 1994 08:49:37 GMT",
            "Sun, 06 Nov 1994 24:00:00 GMT",
            "1994-11-06T08:49:37Z",
        ] {
            assert_eq!(parse(bad), None, "{bad:?}");
        }
    }
}
