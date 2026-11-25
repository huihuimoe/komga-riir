//! Per-user content restrictions by series age rating and sharing labels.

use std::collections::BTreeSet;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AllowExclude {
    /// Only series rated at or below the age are visible.
    AllowOnly,
    /// Series rated at or above the age are hidden.
    Exclude,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AgeRestriction {
    pub age: i32,
    pub restriction: AllowExclude,
}

/// Restrictions in the normalized form of Kotlin `ContentRestrictions`.
///
/// Labels are lowercased, trimmed, and blank ones dropped; a label that is both allowed and
/// excluded is only excluded.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ContentRestrictions {
    age_restriction: Option<AgeRestriction>,
    labels_allow: BTreeSet<String>,
    labels_exclude: BTreeSet<String>,
}

impl ContentRestrictions {
    #[must_use]
    pub fn new(
        age_restriction: Option<AgeRestriction>,
        labels_allow: impl IntoIterator<Item = String>,
        labels_exclude: impl IntoIterator<Item = String>,
    ) -> Self {
        let labels_exclude = normalize_labels(labels_exclude);
        let labels_allow = normalize_labels(labels_allow)
            .difference(&labels_exclude)
            .cloned()
            .collect();
        Self {
            age_restriction,
            labels_allow,
            labels_exclude,
        }
    }

    #[must_use]
    pub fn age_restriction(&self) -> Option<AgeRestriction> {
        self.age_restriction
    }

    #[must_use]
    pub fn labels_allow(&self) -> &BTreeSet<String> {
        &self.labels_allow
    }

    #[must_use]
    pub fn labels_exclude(&self) -> &BTreeSet<String> {
        &self.labels_exclude
    }

    #[must_use]
    pub fn is_restricted(&self) -> bool {
        self.age_restriction.is_some()
            || !self.labels_allow.is_empty()
            || !self.labels_exclude.is_empty()
    }
}

/// Kotlin `lowerNotBlank`: lowercase, trim, then drop whitespace-only labels. Approved
/// difference: trimming follows Java `String.trim` (characters up to `' '`), while Kotlin's
/// `trim()` also strips Unicode whitespace such as `\u{3000}` at either end.
pub(crate) fn normalize_labels(labels: impl IntoIterator<Item = String>) -> BTreeSet<String> {
    labels
        .into_iter()
        .map(|label| {
            label
                .to_lowercase()
                .trim_matches(|character| character <= ' ')
                .to_owned()
        })
        .filter(|label| !label.chars().all(char::is_whitespace))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::ContentRestrictions;

    #[test]
    fn labels_are_normalized_and_exclusion_wins() {
        let restrictions = ContentRestrictions::new(
            None,
            [
                " Kids ".to_owned(),
                "SHARED".to_owned(),
                "\u{3000}".to_owned(),
            ],
            ["shared".to_owned(), "\t".to_owned()],
        );
        assert_eq!(
            restrictions.labels_allow().iter().collect::<Vec<_>>(),
            ["kids"]
        );
        assert_eq!(
            restrictions.labels_exclude().iter().collect::<Vec<_>>(),
            ["shared"]
        );
        assert!(restrictions.is_restricted());
        assert!(!ContentRestrictions::default().is_restricted());
    }
}
