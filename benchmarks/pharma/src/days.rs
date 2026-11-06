//! Calendar dates as days since 1970-01-01, from and to `YYYY-MM-DD` (Howard Hinnant's civil
//! algorithms), for shelf life and delivery dates.

const ERA_DAYS: i64 = 146_097;
const EPOCH_SHIFT: i64 = 719_468;

/// The day of `text`, which starts with `YYYY-MM-DD` (a datetime's date counts); `None` otherwise.
pub fn day(text: &str) -> Option<i64> {
    let date = text.get(..10)?;
    let mut parts = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, d) = (parts.next()??, parts.next()??, parts.next()??);
    if date.as_bytes()[4] != b'-' || !(1..=12).contains(&m) || !(1..=31).contains(&d) {
        return None;
    }
    let y = if m <= 2 { y - 1 } else { y };
    let era = y.div_euclid(400);
    let yoe = y.rem_euclid(400);
    let mp = if m > 2 { m - 3 } else { m + 9 };
    let doy = (153 * mp + 2) / 5 + d - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    Some(era * ERA_DAYS + doe - EPOCH_SHIFT)
}

/// `YYYY-MM-DD` of a day.
pub fn iso(day: i64) -> String {
    let z = day + EPOCH_SHIFT;
    let era = z.div_euclid(ERA_DAYS);
    let doe = z.rem_euclid(ERA_DAYS);
    let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = yoe + era * 400 + i64::from(m <= 2);
    format!("{y:04}-{m:02}-{d:02}")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dates_go_both_ways() {
        assert_eq!(day("1970-01-01"), Some(0));
        assert_eq!(day("2024-02-29"), Some(19_782));
        assert_eq!(iso(19_783), "2024-03-01");
        assert_eq!(day("2026-11-09 06:00:00"), day("2026-11-09"));
        let (a, b) = (day("2026-11-10").unwrap(), day("2027-03-05").unwrap());
        assert_eq!(b - a, 115);
        assert_eq!(day("soon"), None);
        assert_eq!(day("2026-13-01"), None);
        assert_eq!(iso(day("2028-10-18").unwrap()), "2028-10-18");
    }
}
