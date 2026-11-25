//! Kotlin `R2Progression` as the progression routes read and write it.

use jiff::Timestamp;
use jiff::tz::TimeZone;
use komga_types::{R2Device, R2Locator, R2Progression, ReadProgress};
use serde::{Deserialize, Serialize, Serializer};

use crate::mvc::jackson_date_time;

#[derive(Debug, Serialize, Deserialize)]
pub struct R2ProgressionDto {
    #[serde(
        serialize_with = "utc",
        deserialize_with = "jackson_date_time::deserialize_instant"
    )]
    modified: Timestamp,
    device: R2DeviceDto,
    locator: R2Locator,
}

#[derive(Debug, Serialize, Deserialize)]
struct R2DeviceDto {
    id: String,
    name: String,
}

/// `toUTCZoned()`.
fn utc<S: Serializer>(modified: &Timestamp, serializer: S) -> Result<S::Ok, S::Error> {
    serializer.serialize_str(&jackson_date_time::iso_offset_date_time(
        &modified.to_zoned_datetime(TimeZone::UTC),
    ))
}

/// Kotlin `ReadProgress.toR2Progression`: a progress without a locator has an empty one.
impl From<ReadProgress> for R2ProgressionDto {
    fn from(progress: ReadProgress) -> Self {
        Self {
            modified: progress.read_date,
            device: R2DeviceDto {
                id: progress.device_id,
                name: progress.device_name,
            },
            locator: progress.locator.unwrap_or_default(),
        }
    }
}

impl From<R2ProgressionDto> for R2Progression {
    fn from(dto: R2ProgressionDto) -> Self {
        Self {
            modified: dto.modified,
            device: R2Device {
                id: dto.device.id,
                name: dto.device.name,
            },
            locator: dto.locator,
        }
    }
}
