//! Generated thumbnail sizes (Kotlin `ThumbnailSize`).

use std::str::FromStr;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum ThumbnailSize {
    #[default]
    Default,
    Medium,
    Large,
    XLarge,
}

impl ThumbnailSize {
    /// The enum constant name, as persisted in `SERVER_SETTINGS`.
    #[must_use]
    pub const fn name(self) -> &'static str {
        match self {
            Self::Default => "DEFAULT",
            Self::Medium => "MEDIUM",
            Self::Large => "LARGE",
            Self::XLarge => "XLARGE",
        }
    }

    /// The longest side of a generated thumbnail, in pixels.
    #[must_use]
    pub const fn max_edge(self) -> u32 {
        match self {
            Self::Default => 300,
            Self::Medium => 600,
            Self::Large => 900,
            Self::XLarge => 1200,
        }
    }
}

impl FromStr for ThumbnailSize {
    type Err = ();

    /// Exact constant name, like Kotlin `ThumbnailSize.valueOf`.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        match name {
            "DEFAULT" => Ok(Self::Default),
            "MEDIUM" => Ok(Self::Medium),
            "LARGE" => Ok(Self::Large),
            "XLARGE" => Ok(Self::XLarge),
            _ => Err(()),
        }
    }
}
