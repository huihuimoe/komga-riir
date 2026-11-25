//! jOOQ's SQLite text form of Kotlin `LocalDateTime` values (`java.sql.Timestamp.toString()`):
//! `yyyy-MM-dd HH:mm:ss.f`, with the fraction trimmed to at least one digit.

use jiff::Timestamp;
use jiff::civil::{Date, DateTime};
use jiff::tz::TimeZone;
use sea_query::{Nullable, Value};
use sqlx::error::BoxDynError;
use sqlx::sqlite::{SqliteTypeInfo, SqliteValueRef};
use sqlx::{Decode, Sqlite, Type};

/// A `LocalDateTime` column value.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SqlDateTime(pub(crate) DateTime);

/// A `LocalDate` column value, which jOOQ writes as an ISO date.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SqlDate(pub(crate) Date);

macro_rules! sql_text {
    ($type:ident, $format:expr) => {
        impl Type<Sqlite> for $type {
            fn type_info() -> SqliteTypeInfo {
                <str as Type<Sqlite>>::type_info()
            }
        }

        impl<'r> Decode<'r, Sqlite> for $type {
            fn decode(value: SqliteValueRef<'r>) -> Result<Self, BoxDynError> {
                Ok(Self(<&str as Decode<Sqlite>>::decode(value)?.parse()?))
            }
        }

        impl From<$type> for Value {
            fn from(value: $type) -> Self {
                Self::String(Some($format(value.0)))
            }
        }

        impl Nullable for $type {
            fn null() -> Value {
                Value::String(None)
            }
        }
    };
}

sql_text!(SqlDateTime, format);
sql_text!(SqlDate, |date: Date| date.to_string());

fn format(value: DateTime) -> String {
    let nanos = format!("{:09}", value.subsec_nanosecond());
    let fraction = nanos.trim_end_matches('0');
    format!(
        "{} {:02}:{:02}:{:02}.{}",
        value.date(),
        value.hour(),
        value.minute(),
        value.second(),
        if fraction.is_empty() { "0" } else { fraction }
    )
}

/// `LocalDateTime.now(ZoneId.of("Z"))`, which Kotlin writes to `LAST_MODIFIED_DATE`.
pub(crate) fn now_utc() -> SqlDateTime {
    SqlDateTime(Timestamp::now().to_zoned_datetime(TimeZone::UTC).datetime())
}

#[cfg(test)]
mod tests {
    use jiff::civil::date;

    use jiff::civil::DateTime;

    use super::format;

    #[test]
    fn round_trips_the_jooq_form() {
        // Values written by jOOQ 3.19.35 with sqlite-jdbc 3.53.4.0.
        for (value, text) in [
            (
                date(2024, 1, 2).at(3, 4, 5, 678_900_000),
                "2024-01-02 03:04:05.6789",
            ),
            (date(2024, 1, 2).at(3, 4, 5, 0), "2024-01-02 03:04:05.0"),
        ] {
            assert_eq!(format(value), text);
            assert_eq!(text.parse::<DateTime>().expect("jOOQ text"), value);
        }
        assert_eq!(
            "2024-01-02 03:04:05"
                .parse::<DateTime>()
                .expect("SQLite CURRENT_TIMESTAMP"),
            date(2024, 1, 2).at(3, 4, 5, 0)
        );
    }
}
