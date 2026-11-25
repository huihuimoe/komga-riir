//! Kotlin `KomgaSyncTokenGenerator`: the `x-kobo-synctoken` header carrying the sync anchors.

use base64::Engine;
use base64::alphabet::STANDARD;
use base64::engine::DecodePaddingMode;
use base64::engine::general_purpose::{GeneralPurpose, GeneralPurposeConfig};
use komga_types::{SyncAnchors, SyncPointId};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use tracing::warn;

const PREFIX: &str = "KOMGA.";

/// Java's basic Base64: encodes without padding, decodes with or without.
const BASE64: GeneralPurpose = GeneralPurpose::new(
    &STANDARD,
    GeneralPurposeConfig::new()
        .with_encode_padding(false)
        .with_decode_padding_mode(DecodePaddingMode::Indifferent),
);

/// Kotlin `KomgaSyncToken`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase", default)]
pub(super) struct SyncToken {
    version: i32,
    /// The Kobo store's own token, kept for the store proxy.
    pub raw_kobo_sync_token: String,
    ongoing_sync_point_id: Option<SyncPointId>,
    last_successful_sync_point_id: Option<SyncPointId>,
}

impl Default for SyncToken {
    fn default() -> Self {
        Self {
            version: 1,
            raw_kobo_sync_token: String::new(),
            ongoing_sync_point_id: None,
            last_successful_sync_point_id: None,
        }
    }
}

impl SyncToken {
    /// A Komga token is `KOMGA.` and Base64 JSON, a Calibre-Web token Base64 JSON holding the
    /// store token, and a Kobo store token `base64.base64`. Anything else is a fresh token.
    pub fn parse(header: &str) -> Self {
        if let Some(komga) = header.strip_prefix(PREFIX) {
            return BASE64
                .decode(komga)
                .ok()
                .and_then(|json| serde_json::from_slice(&json).ok())
                .unwrap_or_default();
        }
        if header.contains('.') {
            return Self::store(header.to_owned());
        }
        let calibre_web = BASE64
            .decode(header)
            .ok()
            .and_then(|json| serde_json::from_slice::<Value>(&json).ok())
            .and_then(|json| {
                json["data"]["raw_kobo_store_token"]
                    .as_str()
                    .map(str::to_owned)
            });
        match calibre_web {
            Some(store_token) => Self::store(store_token),
            None => {
                warn!("Failed to parse potential CalibreWeb token");
                Self::default()
            }
        }
    }

    fn store(raw_kobo_sync_token: String) -> Self {
        Self {
            raw_kobo_sync_token,
            ..Self::default()
        }
    }

    pub fn encode(&self) -> String {
        let json = serde_json::to_vec(self).expect("A sync token serializes");
        format!("{PREFIX}{}", BASE64.encode(json))
    }

    pub fn anchors(&self) -> SyncAnchors {
        SyncAnchors {
            ongoing: self.ongoing_sync_point_id.clone(),
            last_synced: self.last_successful_sync_point_id.clone(),
        }
    }

    pub fn with_anchors(self, anchors: SyncAnchors) -> Self {
        Self {
            ongoing_sync_point_id: anchors.ongoing,
            last_successful_sync_point_id: anchors.last_synced,
            ..self
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tokens_of_komga_calibre_web_and_the_store_are_recognized() {
        let anchors = SyncAnchors {
            ongoing: None,
            last_synced: Some(SyncPointId::from("point".to_owned())),
        };
        let komga = SyncToken::default().with_anchors(anchors.clone()).encode();
        assert!(!komga.ends_with('='), "{komga}");
        assert_eq!(SyncToken::parse(&komga).anchors(), anchors);
        let padded = format!(
            "{PREFIX}{}",
            base64::prelude::BASE64_STANDARD.encode(r#"{"ongoingSyncPointId":"a"}"#)
        );
        assert_eq!(
            SyncToken::parse(&padded).anchors().ongoing,
            Some(SyncPointId::from("a".to_owned()))
        );

        let calibre_web = BASE64.encode(r#"{"data":{"raw_kobo_store_token":"store.token"}}"#);
        assert_eq!(
            SyncToken::parse(&calibre_web).raw_kobo_sync_token,
            "store.token"
        );
        assert_eq!(
            SyncToken::parse("store.token").raw_kobo_sync_token,
            "store.token"
        );
        assert_eq!(SyncToken::parse("garbage"), SyncToken::default());
        assert_eq!(SyncToken::parse("KOMGA.garbage"), SyncToken::default());
    }
}
