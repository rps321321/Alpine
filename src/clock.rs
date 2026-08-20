use std::time::{SystemTime, UNIX_EPOCH};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct UtcTimestamp {
    pub(crate) epoch_seconds: u64,
    pub(crate) nanoseconds: u32,
}

impl UtcTimestamp {
    pub(crate) fn now() -> Result<Self, String> {
        let duration = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|_| "system clock predates the Unix epoch".to_owned())?;
        Ok(Self {
            epoch_seconds: duration.as_secs(),
            nanoseconds: duration.subsec_nanos(),
        })
    }

    pub(crate) fn compact(self) -> String {
        let (year, month, day, hour, minute, second) = self.parts();
        format!("{year:04}{month:02}{day:02}T{hour:02}{minute:02}{second:02}Z")
    }

    pub(crate) fn rfc3339(self) -> String {
        let (year, month, day, hour, minute, second) = self.parts();
        if self.nanoseconds == 0 {
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
        } else {
            let fraction = format!("{:09}", self.nanoseconds)
                .trim_end_matches('0')
                .to_owned();
            format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}.{fraction}Z")
        }
    }

    fn parts(self) -> (i64, u32, u32, u32, u32, u32) {
        let seconds_per_day = 86_400_u64;
        let days = (self.epoch_seconds / seconds_per_day) as i64;
        let day_seconds = self.epoch_seconds % seconds_per_day;
        let (year, month, day) = civil_from_days(days);
        (
            year,
            month,
            day,
            (day_seconds / 3_600) as u32,
            ((day_seconds % 3_600) / 60) as u32,
            (day_seconds % 60) as u32,
        )
    }
}

fn civil_from_days(days_since_epoch: i64) -> (i64, u32, u32) {
    let shifted = days_since_epoch + 719_468;
    let era = if shifted >= 0 {
        shifted
    } else {
        shifted - 146_096
    } / 146_097;
    let day_of_era = shifted - era * 146_097;
    let year_of_era =
        (day_of_era - day_of_era / 1_460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let mut year = year_of_era + era * 400;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let month_prime = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * month_prime + 2) / 5 + 1;
    let month = month_prime + if month_prime < 10 { 3 } else { -9 };
    if month <= 2 {
        year += 1;
    }
    (year, month as u32, day as u32)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn formatter_handles_epoch_leap_day_and_fraction() {
        assert_eq!(
            UtcTimestamp {
                epoch_seconds: 0,
                nanoseconds: 0
            }
            .rfc3339(),
            "1970-01-01T00:00:00Z"
        );
        let leap_day = UtcTimestamp {
            epoch_seconds: 951_782_400,
            nanoseconds: 123_400_000,
        };
        assert_eq!(leap_day.rfc3339(), "2000-02-29T00:00:00.1234Z");
        assert_eq!(leap_day.compact(), "20000229T000000Z");
    }
}
