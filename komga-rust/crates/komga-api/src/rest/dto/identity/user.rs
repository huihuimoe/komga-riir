//! User request and response DTOs.

use std::collections::BTreeSet;

use komga_service::{LibrarySharing, NewUser, UserUpdate};
use komga_types::{
    AgeRestriction, AllowExclude, ContentRestrictions, KomgaUser, LibraryId, UserId, UserRole,
};
use serde::{Deserialize, Serialize};

use crate::error::Violation;
use crate::rest::dto::present;
use crate::rest::validation;

/// User creation request DTO.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserCreationDto {
    pub email: String,
    pub password: String,
    pub roles: Option<Vec<String>>,
    pub age_restriction: Option<AgeRestrictionUpdateDto>,
    pub labels_allow: Option<Vec<String>>,
    pub labels_exclude: Option<Vec<String>>,
    pub shared_libraries: Option<SharedLibrariesUpdateDto>,
}

impl UserCreationDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::email("email", &self.email, &mut violations);
        validation::not_blank("password", &self.password, &mut violations);
        if let Some(age_restriction) = &self.age_restriction {
            age_restriction.validate(&mut violations);
        }
        violations
    }

    /// Absent sharing means all libraries and absent restrictions mean none, as before those
    /// fields existed.
    pub(crate) fn into_new_user(self) -> NewUser {
        NewUser {
            roles: parse_roles(self.roles.as_deref().unwrap_or_default()),
            library_sharing: self
                .shared_libraries
                .map(SharedLibrariesUpdateDto::into_sharing)
                .unwrap_or_default(),
            restrictions: ContentRestrictions::new(
                self.age_restriction
                    .and_then(AgeRestrictionUpdateDto::into_domain),
                self.labels_allow.unwrap_or_default(),
                self.labels_exclude.unwrap_or_default(),
            ),
            email: self.email,
            password: self.password,
        }
    }
}

/// User password update request DTO.
#[derive(Debug, Clone, Deserialize)]
pub struct PasswordUpdateDto {
    pub password: String,
}

impl PasswordUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        validation::not_blank("password", &self.password, &mut violations);
        violations
    }
}

/// User PATCH request DTO; only fields present in the body change.
///
/// Explicit `null` for `roles` or `sharedLibraries` is treated as absent; Kotlin dereferences
/// it and fails with 500.
#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct UserUpdateDto {
    pub roles: Option<Vec<String>>,
    pub shared_libraries: Option<SharedLibrariesUpdateDto>,
    #[serde(default, deserialize_with = "present")]
    pub age_restriction: Option<Option<AgeRestrictionUpdateDto>>,
    #[serde(default, deserialize_with = "present")]
    pub labels_allow: Option<Option<Vec<String>>>,
    #[serde(default, deserialize_with = "present")]
    pub labels_exclude: Option<Option<Vec<String>>>,
}

impl UserUpdateDto {
    pub(crate) fn violations(&self) -> Vec<Violation> {
        let mut violations = Vec::new();
        if let Some(Some(age_restriction)) = &self.age_restriction {
            age_restriction.validate(&mut violations);
        }
        violations
    }

    pub(crate) fn into_update(self) -> UserUpdate {
        let labels = |labels: Option<Vec<String>>| labels.unwrap_or_default().into_iter().collect();
        UserUpdate {
            roles: self.roles.as_deref().map(parse_roles),
            library_sharing: self
                .shared_libraries
                .map(SharedLibrariesUpdateDto::into_sharing),
            age_restriction: self
                .age_restriction
                .map(|age| age.and_then(AgeRestrictionUpdateDto::into_domain)),
            labels_allow: self.labels_allow.map(labels),
            labels_exclude: self.labels_exclude.map(labels),
        }
    }
}

#[derive(Debug, Clone, Deserialize)]
pub struct AgeRestrictionUpdateDto {
    pub age: i32,
    pub restriction: AllowExcludeDto,
}

impl AgeRestrictionUpdateDto {
    fn validate(&self, violations: &mut Vec<Violation>) {
        validation::positive_or_zero("ageRestriction.age", self.age, violations);
    }

    /// `NONE` clears the restriction.
    fn into_domain(self) -> Option<AgeRestriction> {
        let restriction = match self.restriction {
            AllowExcludeDto::AllowOnly => AllowExclude::AllowOnly,
            AllowExcludeDto::Exclude => AllowExclude::Exclude,
            AllowExcludeDto::None => return None,
        };
        Some(AgeRestriction {
            age: self.age,
            restriction,
        })
    }
}

#[derive(Debug, Clone, Copy, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum AllowExcludeDto {
    AllowOnly,
    Exclude,
    None,
}

#[derive(Debug, Clone, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SharedLibrariesUpdateDto {
    pub all: bool,
    pub library_ids: Vec<LibraryId>,
}

impl SharedLibrariesUpdateDto {
    fn into_sharing(self) -> LibrarySharing {
        LibrarySharing {
            all: self.all,
            library_ids: self.library_ids.into_iter().collect(),
        }
    }
}

/// User DTO matching Kotlin baseline UserDto.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct UserDto {
    pub id: UserId,
    pub email: String,
    pub roles: Vec<String>,
    pub shared_all_libraries: bool,
    pub shared_libraries_ids: Vec<LibraryId>,
    pub labels_allow: Vec<String>,
    pub labels_exclude: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub age_restriction: Option<AgeRestrictionDto>,
}

#[derive(Debug, Clone, Serialize)]
pub struct AgeRestrictionDto {
    pub age: i32,
    pub restriction: &'static str,
}

impl From<&KomgaUser> for UserDto {
    fn from(user: &KomgaUser) -> Self {
        // Every authenticated principal also holds the implicit `USER` authority.
        let roles = user
            .roles
            .iter()
            .map(|role| role.as_str())
            .chain(["USER"])
            .map(str::to_owned)
            .collect();
        let age_restriction =
            user.restrictions
                .age_restriction()
                .map(|age_restriction| AgeRestrictionDto {
                    age: age_restriction.age,
                    restriction: match age_restriction.restriction {
                        AllowExclude::AllowOnly => "ALLOW_ONLY",
                        AllowExclude::Exclude => "EXCLUDE",
                    },
                });
        Self {
            id: user.id.clone(),
            email: user.email.clone(),
            roles,
            shared_all_libraries: user.shared_all_libraries,
            shared_libraries_ids: user.shared_library_ids.iter().cloned().collect(),
            labels_allow: user.restrictions.labels_allow().iter().cloned().collect(),
            labels_exclude: user.restrictions.labels_exclude().iter().cloned().collect(),
            age_restriction,
        }
    }
}

impl From<KomgaUser> for UserDto {
    fn from(user: KomgaUser) -> Self {
        Self::from(&user)
    }
}

/// `UserRoles.valuesOf`: exact constant names; unknown names are ignored.
fn parse_roles(roles: &[String]) -> BTreeSet<UserRole> {
    roles.iter().filter_map(|role| role.parse().ok()).collect()
}
