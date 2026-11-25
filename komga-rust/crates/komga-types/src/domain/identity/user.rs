//! Komga user entity and roles.

use std::collections::BTreeSet;
use std::str::FromStr;

use serde::{Deserialize, Serialize};

use super::content_restrictions;
use crate::domain::{AllowExclude, ContentRestrictions};
use crate::ids::{LibraryId, UserId};

/// Role assigned to a Komga user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum UserRole {
    Admin,
    FileDownload,
    PageStreaming,
    KoboSync,
    KoreaderSync,
}

impl UserRole {
    /// Every role, as granted to the user who claims the server.
    pub const ALL: [Self; 5] = [
        Self::Admin,
        Self::FileDownload,
        Self::PageStreaming,
        Self::KoboSync,
        Self::KoreaderSync,
    ];

    /// Baseline enum constant name, used both in storage and in DTOs.
    #[must_use]
    pub fn as_str(self) -> &'static str {
        match self {
            Self::Admin => "ADMIN",
            Self::FileDownload => "FILE_DOWNLOAD",
            Self::PageStreaming => "PAGE_STREAMING",
            Self::KoboSync => "KOBO_SYNC",
            Self::KoreaderSync => "KOREADER_SYNC",
        }
    }
}

/// Error for a name that is not exactly a baseline role constant.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct UnknownUserRole;

impl FromStr for UserRole {
    type Err = UnknownUserRole;

    /// Matches names exactly, like Kotlin `UserRoles.valueOf`.
    fn from_str(name: &str) -> Result<Self, Self::Err> {
        Self::ALL
            .into_iter()
            .find(|role| role.as_str() == name)
            .ok_or(UnknownUserRole)
    }
}

/// Core domain entity representing a Komga user.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct KomgaUser {
    pub id: UserId,
    pub email: String,
    pub password_hash: String,
    pub roles: BTreeSet<UserRole>,
    pub shared_all_libraries: bool,
    /// Libraries shared with the user; ignored while `shared_all_libraries` is set.
    pub shared_library_ids: BTreeSet<LibraryId>,
    pub restrictions: ContentRestrictions,
}

impl KomgaUser {
    #[must_use]
    pub fn is_admin(&self) -> bool {
        self.roles.contains(&UserRole::Admin)
    }

    /// Spring `hasRole`: roles imply no others, so an admin without `PAGE_STREAMING` cannot read.
    #[must_use]
    pub fn has_role(&self, role: UserRole) -> bool {
        self.roles.contains(&role)
    }

    #[must_use]
    pub fn can_access_library(&self, library_id: &LibraryId) -> bool {
        self.shared_all_libraries || self.is_admin() || self.shared_library_ids.contains(library_id)
    }

    /// Kotlin `isContentAllowed` for a series with `age_rating` and `sharing_labels`: an allow
    /// rule (age or label) must match when one is set, and no exclude rule may match.
    #[must_use]
    pub fn is_content_allowed(&self, age_rating: Option<i32>, sharing_labels: &[String]) -> bool {
        let labels = content_restrictions::normalize_labels(sharing_labels.iter().cloned());
        let restrictions = &self.restrictions;
        let age = restrictions.age_restriction();
        let age_allowed = age
            .filter(|age| age.restriction == AllowExclude::AllowOnly)
            .map(|age| age_rating.is_some_and(|rating| rating <= age.age));
        let label_allowed = (!restrictions.labels_allow().is_empty())
            .then(|| !restrictions.labels_allow().is_disjoint(&labels));
        let allowed = match (age_allowed, label_allowed) {
            (None, None) => true,
            (Some(allowed), None) | (None, Some(allowed)) => allowed,
            (Some(age), Some(label)) => age || label,
        };
        let age_denied = age
            .filter(|age| age.restriction == AllowExclude::Exclude)
            .is_some_and(|age| age_rating.is_some_and(|rating| rating >= age.age));
        let label_denied = !restrictions.labels_exclude().is_disjoint(&labels);
        allowed && !age_denied && !label_denied
    }
}

#[cfg(test)]
mod tests {
    use std::collections::BTreeSet;

    use super::{KomgaUser, UserRole};
    use crate::domain::{AgeRestriction, AllowExclude, ContentRestrictions};
    use crate::ids::{LibraryId, UserId};

    fn user(age: Option<(i32, AllowExclude)>, allow: &[&str], exclude: &[&str]) -> KomgaUser {
        let labels = |labels: &[&str]| {
            labels
                .iter()
                .map(|label| (*label).to_owned())
                .collect::<Vec<_>>()
        };
        KomgaUser {
            id: UserId::from("user".to_owned()),
            email: "reader@example.com".to_owned(),
            password_hash: String::new(),
            roles: BTreeSet::from([UserRole::PageStreaming]),
            shared_all_libraries: false,
            shared_library_ids: BTreeSet::from([LibraryId::from("shared".to_owned())]),
            restrictions: ContentRestrictions::new(
                age.map(|(age, restriction)| AgeRestriction { age, restriction }),
                labels(allow),
                labels(exclude),
            ),
        }
    }

    #[test]
    fn content_is_allowed_like_kotlin() {
        use AllowExclude::{AllowOnly, Exclude};

        let kids = [" Kids ".to_owned()];
        for (user, rating, labels, expected) in [
            (user(None, &[], &[]), None, &[][..], true),
            // ALLOW_ONLY needs a rating at or below the age.
            (user(Some((12, AllowOnly)), &[], &[]), Some(12), &[], true),
            (user(Some((12, AllowOnly)), &[], &[]), Some(13), &[], false),
            (user(Some((12, AllowOnly)), &[], &[]), None, &[], false),
            // Either allow rule suffices once both are set; labels compare normalized.
            (
                user(Some((12, AllowOnly)), &["kids"], &[]),
                Some(16),
                &kids[..],
                true,
            ),
            (
                user(Some((12, AllowOnly)), &["kids"], &[]),
                Some(16),
                &[],
                false,
            ),
            // EXCLUDE hides ratings at or above the age; unrated series stay visible.
            (user(Some((16, Exclude)), &[], &[]), Some(16), &[], false),
            (user(Some((16, Exclude)), &[], &[]), None, &[], true),
            // An excluded label wins over an allowed rating.
            (
                user(Some((12, AllowOnly)), &[], &["kids"]),
                Some(6),
                &kids[..],
                false,
            ),
        ] {
            assert_eq!(
                user.is_content_allowed(rating, labels),
                expected,
                "{:?} {rating:?} {labels:?}",
                user.restrictions
            );
        }
    }

    #[test]
    fn libraries_are_shared_explicitly_unless_admin_or_shared_all() {
        let mut reader = user(None, &[], &[]);
        let other = LibraryId::from("other".to_owned());
        assert!(reader.can_access_library(&LibraryId::from("shared".to_owned())));
        assert!(!reader.can_access_library(&other));
        reader.roles.insert(UserRole::Admin);
        assert!(reader.can_access_library(&other));
        assert!(!reader.has_role(UserRole::FileDownload));
    }
}
