//! The file time a scan records and how it is compared.

use std::fs::Metadata;
use std::time::SystemTime;

use jiff::Timestamp;
use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use komga_types::Book;

/// Kotlin `getUpdatedTime`: the later of creation and modification, in `time_zone`. Java reports
/// the modification time where creation time is unsupported.
pub(crate) fn updated_time(metadata: &Metadata, time_zone: &TimeZone) -> DateTime {
    let modified = metadata.modified().unwrap_or(SystemTime::UNIX_EPOCH);
    let updated = metadata
        .created()
        .map_or(modified, |created| created.max(modified));
    Timestamp::try_from(updated)
        .unwrap_or(Timestamp::UNIX_EPOCH)
        .to_zoned_datetime(time_zone.clone())
        .datetime()
}

/// Kotlin `notEquals` with millisecond precision, negated.
pub(crate) fn same_millis(left: DateTime, right: DateTime) -> bool {
    let millis = |value: DateTime| {
        (
            value.date(),
            value.hour(),
            value.minute(),
            value.second(),
            value.subsec_nanosecond() / 1_000_000,
        )
    };
    millis(left) == millis(right)
}

/// Whether the file is the version the scan recorded for `book`: the scan's own change test, where
/// size and time are evidence, not proof, of equal content.
pub(crate) fn is_recorded(metadata: &Metadata, book: &Book) -> bool {
    i64::try_from(metadata.len()) == Ok(book.file_size)
        && same_millis(
            updated_time(metadata, &TimeZone::system()),
            book.file_last_modified,
        )
}
