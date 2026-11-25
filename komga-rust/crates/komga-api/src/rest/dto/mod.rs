//! REST data transfer objects.

mod book;
mod catalog;
mod identity;
mod library;
mod series;
mod server;

pub use book::*;
pub use catalog::*;
pub use identity::*;
pub use library::*;
pub use series::*;
pub use server::*;

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

/// A UTC date time with `@JsonFormat(pattern = "yyyy-MM-dd'T'HH:mm:ss'Z'")`.
#[derive(Debug, Clone, Copy)]
struct JsonDateTime(DateTime);

impl Serialize for JsonDateTime {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.collect_str(&self.0.strftime("%Y-%m-%dT%H:%M:%SZ"))
    }
}

/// A file time, which is in the server's zone, in UTC.
fn file_time_in_utc(value: DateTime) -> DateTime {
    value
        .to_zoned_datetime(TimeZone::system())
        .expect("Stored dates are in range")
        .with_time_zone(TimeZone::UTC)
        .datetime()
}

/// Distinguishes a field sent as `null` (`Some(None)`) from an absent one (`None`).
fn present<'de, D, T>(deserializer: D) -> Result<Option<Option<T>>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer).map(Some)
}
