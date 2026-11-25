//! Jackson's default `ISO_OFFSET_DATE_TIME` form of `ZonedDateTime` values, and its parsing.

use jiff::tz::Offset;
use jiff::{Timestamp, ZonedDateTime};
use serde::{Deserialize, Deserializer};

/// Seconds always, the fraction without trailing zeros, `Z` for UTC, in the value's own zone.
pub(crate) fn iso_offset_date_time(value: &ZonedDateTime) -> String {
    let nanos = format!("{:09}", value.subsec_nanosecond());
    let fraction = nanos.trim_end_matches('0');
    let fraction = if fraction.is_empty() {
        String::new()
    } else {
        format!(".{fraction}")
    };
    format!(
        "{}T{:02}:{:02}:{:02}{fraction}{}",
        value.date(),
        value.hour(),
        value.minute(),
        value.second(),
        offset_id(value.offset())
    )
}

/// `ZoneOffset.getId`: `Z`, or `±HH:MM` with `:SS` when there are seconds.
fn offset_id(offset: Offset) -> String {
    let seconds = offset.seconds();
    if seconds == 0 {
        return "Z".to_owned();
    }
    let sign = if seconds < 0 { '-' } else { '+' };
    let absolute = seconds.unsigned_abs();
    let (hours, minutes, rest) = (absolute / 3600, absolute / 60 % 60, absolute % 60);
    if rest == 0 {
        format!("{sign}{hours:02}:{minutes:02}")
    } else {
        format!("{sign}{hours:02}:{minutes:02}:{rest:02}")
    }
}

/// A `ZonedDateTime` request value as the instant it names: an ISO 8601 date time with an
/// offset, optionally followed by a bracketed zone.
pub(crate) fn deserialize_instant<'de, D>(deserializer: D) -> Result<Timestamp, D::Error>
where
    D: Deserializer<'de>,
{
    let text = String::deserialize(deserializer)?;
    text.parse().map_err(serde::de::Error::custom)
}

#[cfg(test)]
mod tests {
    use jiff::Timestamp;
    use jiff::civil::date;
    use jiff::tz::{Offset, TimeZone};

    use super::{iso_offset_date_time, offset_id};

    #[test]
    fn dates_are_formatted_like_jackson() {
        let zoned = |nanos, zone| {
            date(2024, 1, 2)
                .at(3, 4, 0, nanos)
                .to_zoned_datetime(zone)
                .expect("In range")
        };
        assert_eq!(
            iso_offset_date_time(&zoned(120_000_000, TimeZone::UTC)),
            "2024-01-02T03:04:00.12Z"
        );
        let tokyo = TimeZone::fixed(Offset::from_seconds(9 * 3600).expect("Offset"));
        assert_eq!(
            iso_offset_date_time(&zoned(0, tokyo)),
            "2024-01-02T03:04:00+09:00"
        );

        for (seconds, expected) in [
            (0, "Z"),
            (9 * 3600, "+09:00"),
            (-(5 * 3600 + 30 * 60), "-05:30"),
            (3600 + 1, "+01:00:01"),
        ] {
            let offset = Offset::from_seconds(seconds).expect("Offset");
            assert_eq!(offset_id(offset), expected);
        }
    }

    #[test]
    fn instants_parse_with_any_offset_or_zone() {
        let expected: Timestamp = "2024-01-02T03:04:05.5Z".parse().expect("UTC");
        for text in [
            "2024-01-02T03:04:05.5Z",
            "2024-01-02T12:04:05.500+09:00",
            "2024-01-02T04:04:05.5+01:00[Europe/Paris]",
        ] {
            let parsed: Timestamp =
                serde_json::from_value::<Wrapper>(serde_json::json!({"at": text}))
                    .expect(text)
                    .at;
            assert_eq!(parsed, expected, "{text}");
        }
        assert!(
            serde_json::from_value::<Wrapper>(serde_json::json!({"at": "2024-01-02T03:04:05"}))
                .is_err()
        );
    }

    #[derive(serde::Deserialize)]
    struct Wrapper {
        #[serde(deserialize_with = "super::deserialize_instant")]
        at: Timestamp,
    }
}
