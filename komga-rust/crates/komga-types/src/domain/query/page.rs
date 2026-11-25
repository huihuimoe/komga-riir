//! Page requests and results for list queries.

/// Requested slice of a list query.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PageRequest {
    Paged { number: u32, size: u32 },
    Unpaged,
}

impl PageRequest {
    /// Row offset of a paged request.
    #[must_use]
    pub fn offset(self) -> u64 {
        match self {
            Self::Paged { number, size } => u64::from(number) * u64::from(size),
            Self::Unpaged => 0,
        }
    }
}

/// Items of the requested page and the total across all pages.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PageResult<T> {
    pub items: Vec<T>,
    pub total_elements: u64,
}

/// Spring Data `Sort.Direction`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SortDirection {
    Asc,
    Desc,
}

/// One `Sort.Order` whose property a query knows as `key`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SortOrder<K> {
    pub key: K,
    pub direction: SortDirection,
}
