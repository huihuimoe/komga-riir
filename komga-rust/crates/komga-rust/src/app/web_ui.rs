//! The UIs built from `komga-webui` and `next-ui`: release builds embed them, other builds read
//! them from disk and serve no UI when it was not built.

use komga_api::{WebAsset, WebUi};
use rust_embed::{EmbeddedFile, RustEmbed};

/// Without source maps: they are 14.5 MiB of the 26.7 MiB build.
#[derive(RustEmbed)]
#[folder = "../../../komga-webui/dist"]
#[exclude = "*.map"]
#[allow_missing = true]
struct Classic;

#[derive(RustEmbed)]
#[folder = "../../../next-ui/dist"]
#[allow_missing = true]
struct Next;

pub(crate) const WEB_UI: WebUi = WebUi {
    classic: |name| Classic::get(name).map(asset),
    next: |name| Next::get(name).map(asset),
};

fn asset(file: EmbeddedFile) -> WebAsset {
    WebAsset {
        mime: file.metadata.mimetype().to_owned().into(),
        last_modified: file.metadata.last_modified(),
        data: file.data,
    }
}
