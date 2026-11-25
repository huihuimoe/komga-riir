//! Library root for `komga-rust`.

pub mod admin;
pub mod app;
pub mod config;
pub mod logging;

use komga_api::{BuildInfo, GitInfo};

/// Multi-threaded Tokio runtime configured with the unstable scheduler and I/O drivers
/// (`io_uring` on Linux via `enable_all`, eager driver handoff, sharded blocking queue, and the
/// alternative timer). Shared by the server binary and the integration test harness.
#[must_use]
pub fn runtime_builder() -> tokio::runtime::Builder {
    let mut builder = tokio::runtime::Builder::new_multi_thread();
    builder
        .enable_all()
        .enable_eager_driver_handoff()
        .enable_sharded_blocking_queue()
        .enable_alt_timer();
    builder
}

/// Release builds inject the tag through `VERSION_NEXT`; local builds fall back to the crate
/// version.
pub const BUILD_INFO: BuildInfo = BuildInfo {
    version: match option_env!("VERSION_NEXT") {
        Some(version) => {
            let trimmed = version.trim_ascii();
            if trimmed.is_empty() {
                env!("CARGO_PKG_VERSION")
            } else {
                trimmed
            }
        }
        None => env!("CARGO_PKG_VERSION"),
    },
    git: match option_env!("GIT_COMMIT_ID") {
        Some(commit_id) => Some(GitInfo {
            branch: option_env!("GIT_BRANCH"),
            commit_id,
            commit_time: option_env!("GIT_COMMIT_TIME"),
        }),
        None => None,
    },
};
