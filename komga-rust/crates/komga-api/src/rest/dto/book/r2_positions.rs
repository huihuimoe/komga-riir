//! Kotlin `R2Positions`.

use komga_types::R2Locator;
use serde::Serialize;

/// The locators are serialized as the Kotlin domain `R2Locator` is.
#[derive(Debug, Serialize)]
pub struct R2Positions {
    total: usize,
    positions: Vec<R2Locator>,
}

impl R2Positions {
    #[must_use]
    pub fn new(positions: Vec<R2Locator>) -> Self {
        Self {
            total: positions.len(),
            positions,
        }
    }
}
