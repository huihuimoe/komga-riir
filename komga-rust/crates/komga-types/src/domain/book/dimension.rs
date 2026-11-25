//! Pixel size of an image.

use serde::{Deserialize, Serialize};

/// Width and height of an image in pixels.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct Dimension {
    pub width: u32,
    pub height: u32,
}
