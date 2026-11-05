//! Calendar dates as `YYYY-MM-DD`, for dates in the seed relative to the plant's first day.
/// Days in a 400-year cycle, and in the part of it before a March-based year's offset.
const ERA_DAYS: i64 = 146_097;
const EPOCH_SHIFT: i64 = 719_468;

/// A day as days since 1970-01-01.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub struct Day(i64);

impl Day {
    /// The plant's first day, 2026-03-10: the `plant_start` of the hub and the plant, so the ERP,
    /// the CRM and the plant keep one calendar.
    pub fn plant() -> Day {
        Day(20_522)
    }

    /// `n` days later (earlier when negative).
    pub fn plus(self, n: i64) -> Day {
        Day(self.0 + n)
    }

    /// `(year, month, day)`, by the civil-from-days algorithm (Howard Hinnant's).
    fn civil(self) -> (i64, i64, i64) {
        let z = self.0 + EPOCH_SHIFT;
        let era = z.div_euclid(ERA_DAYS);
        let doe = z.rem_euclid(ERA_DAYS);
        let yoe = (doe - doe / 1460 + doe / 36524 - doe / 146_096) / 365;
        let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
        let mp = (5 * doy + 2) / 153;
        let day = doy - (153 * mp + 2) / 5 + 1;
        let month = if mp < 10 { mp + 3 } else { mp - 9 };
        let year = yoe + era * 400 + i64::from(month <= 2);
        (year, month, day)
    }

    pub fn year(self) -> i64 {
        self.civil().0
    }

    pub fn iso(self) -> String {
        let (y, m, d) = self.civil();
        format!("{y:04}-{m:02}-{d:02}")
    }
}

#[cfg(test)]
impl Day {
    /// The day with this many days since the epoch, for tests.
    pub fn from_epoch(days: i64) -> Day {
        Day(days)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_become_calendar_dates() {
        assert_eq!(Day::from_epoch(0).iso(), "1970-01-01");
        assert_eq!(Day::from_epoch(20_000).iso(), "2024-10-04");
        // 2024 is a leap year; 1900 and 2100 are not.
        assert_eq!(Day::from_epoch(19_782).iso(), "2024-02-29");
        assert_eq!(Day::from_epoch(19_782).plus(1).iso(), "2024-03-01");
        assert_eq!(Day::from_epoch(-1).iso(), "1969-12-31");
        assert_eq!(Day::from_epoch(19_782).year(), 2024);
        assert_eq!(Day::plant().iso(), "2026-03-10");
    }
}
