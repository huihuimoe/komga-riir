//! Kotlin `serializeJsonGz`/`deserializeJsonGz`: values stored as JSON compressed in one gzip
//! member.

use flate2::Compression;
use flate2::read::GzDecoder;
use flate2::write::GzEncoder;
use serde::Serialize;
use serde::de::DeserializeOwned;

pub(crate) fn serialize<T: Serialize>(value: &T) -> Vec<u8> {
    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    serde_json::to_writer(&mut encoder, value)
        .expect("The value serializes to an in-memory buffer");
    encoder
        .finish()
        .expect("Compressing to an in-memory buffer does not fail")
}

pub(crate) fn deserialize<T: DeserializeOwned>(blob: &[u8]) -> Result<T, serde_json::Error> {
    serde_json::from_reader(GzDecoder::new(blob))
}
