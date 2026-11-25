//! What a user may see when content is queried.

use std::collections::BTreeSet;

use crate::domain::{ContentRestrictions, KomgaUser};
use crate::ids::LibraryId;

/// Visibility scope of Kotlin `SearchContext`, derived only from an authenticated user.
///
/// There is no other constructor, so callers cannot widen a scope from request input.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContentScope {
    library_ids: Option<BTreeSet<LibraryId>>,
    restrictions: ContentRestrictions,
}

impl ContentScope {
    /// Admins and users shared on all libraries see every library; others only their shared
    /// libraries, possibly none. Content restrictions apply to admins too.
    #[must_use]
    pub fn for_user(user: &KomgaUser) -> Self {
        let library_ids = if user.shared_all_libraries || user.is_admin() {
            None
        } else {
            Some(user.shared_library_ids.clone())
        };
        Self {
            library_ids,
            restrictions: user.restrictions.clone(),
        }
    }

    /// `None` means every library.
    #[must_use]
    pub fn library_ids(&self) -> Option<&BTreeSet<LibraryId>> {
        self.library_ids.as_ref()
    }

    #[must_use]
    pub fn restrictions(&self) -> &ContentRestrictions {
        &self.restrictions
    }
}
