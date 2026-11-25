//! Kotlin `SearchCondition.Series`: filters of a series list query.

use fancy_regex::{Regex, RegexBuilder};
use serde::Deserialize;

use crate::domain::{
    AuthorMatch, BooleanOperator, DateOperator, Equality, EqualityNullable, NotableEntityMatch,
    NumericNullable, ReadStatus, SeriesStatus, StringOperator,
};

/// A series condition; its JSON form is an object whose single property names the condition.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum SeriesCondition {
    AnyOf(Vec<SeriesCondition>),
    AllOf(Vec<SeriesCondition>),
    LibraryId(Equality<String>),
    CollectionId(Equality<String>),
    Deleted(BooleanOperator),
    /// The total book count is set and equals (`isTrue`) or differs from the book count.
    Complete(BooleanOperator),
    /// next-ui's recently added series section sends `oneshot`, which the baseline accepts
    /// through Jackson's case-insensitive property matching.
    #[serde(alias = "oneshot")]
    OneShot(BooleanOperator),
    Title(StringOperator),
    TitleSort(StringOperator),
    /// The release date of the book metadata aggregation.
    ReleaseDate(DateOperator),
    /// Series tags and book tags alike.
    Tag(EqualityNullable<String>),
    SharingLabel(EqualityNullable<String>),
    Publisher(Equality<String>),
    Language(Equality<String>),
    Genre(EqualityNullable<String>),
    AgeRating(NumericNullable<i32>),
    ReadStatus(Equality<ReadStatus>),
    SeriesStatus(Equality<SeriesStatus>),
    /// Authors of the book metadata aggregation.
    Author(Equality<AuthorMatch>),
    /// Notable entities of the book metadata aggregation.
    NotableEntity(Equality<NotableEntityMatch>),
    /// The deprecated `search_regex` query parameter; no JSON form.
    #[serde(skip)]
    TitleRegex(TitleRegex),
}

/// Kotlin `SeriesSearch.regexSearch`: a regular expression found anywhere in a title, ignoring
/// case.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TitleRegex {
    pattern: String,
    field: TitleField,
}

/// Kotlin `SearchField`: the metadata title a [`TitleRegex`] searches.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TitleField {
    Title,
    TitleSort,
}

impl TitleRegex {
    /// # Errors
    /// Returns the syntax error of a `pattern` that does not compile.
    pub fn try_new(pattern: String, field: TitleField) -> Result<Self, fancy_regex::Error> {
        Self::compile(&pattern)?;
        Ok(Self { pattern, field })
    }

    /// The regular expression of `pattern`, case-insensitive like Kotlin `IGNORE_CASE`.
    ///
    /// # Errors
    /// Returns the syntax error of a `pattern` that does not compile.
    pub fn compile(pattern: &str) -> Result<Regex, fancy_regex::Error> {
        RegexBuilder::new(pattern).case_insensitive(true).build()
    }

    #[must_use]
    pub fn pattern(&self) -> &str {
        &self.pattern
    }

    #[must_use]
    pub fn field(&self) -> TitleField {
        self.field
    }
}

impl SeriesCondition {
    /// Collections the series must belong to, in the order they appear; the first orders series
    /// by their number in it.
    #[must_use]
    pub fn collections(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        self.collect_collections(&mut ids);
        ids
    }

    /// The library every matching series belongs to, when the condition names exactly one.
    #[must_use]
    pub fn sole_library(&self) -> Option<&str> {
        match self {
            Self::LibraryId(Equality::Is { value }) => Some(value),
            Self::AllOf(conditions) => conditions.iter().find_map(Self::sole_library),
            Self::AnyOf(conditions) => {
                let mut libraries = conditions.iter().map(Self::sole_library);
                let first = libraries.next()??;
                libraries
                    .all(|library| library == Some(first))
                    .then_some(first)
            }
            _ => None,
        }
    }

    fn collect_collections<'condition>(&'condition self, ids: &mut Vec<&'condition str>) {
        match self {
            Self::AnyOf(conditions) | Self::AllOf(conditions) => {
                for condition in conditions {
                    condition.collect_collections(ids);
                }
            }
            Self::CollectionId(Equality::Is { value }) if !ids.contains(&value.as_str()) => {
                ids.push(value);
            }
            _ => {}
        }
    }
}

/// `SeriesDtoDao.sorts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesSort {
    TitleSort,
    CreatedDate,
    LastModifiedDate,
    /// The release date of the book metadata aggregation.
    ReleaseDate,
    /// The user's most recent read in the series.
    ReadDate,
    /// Only applies when the condition selects a collection.
    CollectionNumber,
    Name,
    BooksCount,
    Random,
    /// Only applies with a full-text search: its most relevant hits first.
    Relevance,
}

/// The number of series whose title sort starts with `group` (`SeriesDtoDao.countByFirstCharacter`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GroupCount {
    /// The first character of the title sort, lowercased like SQLite `lower`; empty for an empty
    /// title sort.
    pub group: String,
    pub count: u64,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::SeriesCondition;
    use crate::domain::{BooleanOperator, Equality, NumericNullable, SeriesStatus};

    #[test]
    fn sole_library_needs_every_branch_to_name_the_same_library() {
        let library = |id: &str| SeriesCondition::LibraryId(Equality::Is { value: id.into() });
        let any = |ids: &[&str]| SeriesCondition::AnyOf(ids.iter().map(|id| library(id)).collect());
        let all = |conditions| SeriesCondition::AllOf(conditions);
        assert_eq!(library("a").sole_library(), Some("a"));
        assert_eq!(all(vec![any(&["a", "a"])]).sole_library(), Some("a"));
        assert_eq!(any(&["a", "b"]).sole_library(), None);
        assert_eq!(any(&[]).sole_library(), None);
        let other = SeriesCondition::LibraryId(Equality::IsNot { value: "a".into() });
        assert_eq!(other.sole_library(), None);
        let mixed = SeriesCondition::AnyOf(vec![library("a"), other]);
        assert_eq!(mixed.sole_library(), None);
    }

    #[test]
    fn conditions_read_series_operands() {
        let condition: SeriesCondition = serde_json::from_value(json!({"anyOf": [
            {"collectionId": {"operator": "is", "value": "b"}},
            {"ageRating": {"operator": "isNull"}},
            {"seriesStatus": {"operator": "isNot", "value": "HIATUS"}},
            {"allOf": [{"collectionId": {"operator": "is", "value": "a"}}]},
        ]}))
        .expect("Valid");
        let SeriesCondition::AnyOf(conditions) = &condition else {
            panic!("Parsed {condition:?}");
        };
        assert_eq!(
            conditions[1],
            SeriesCondition::AgeRating(NumericNullable::IsNull)
        );
        assert_eq!(
            conditions[2],
            SeriesCondition::SeriesStatus(Equality::IsNot {
                value: SeriesStatus::Hiatus
            })
        );
        assert_eq!(condition.collections(), ["b", "a"]);
        let lowercase = json!({"oneshot": {"operator": "isFalse"}});
        assert_eq!(
            serde_json::from_value::<SeriesCondition>(lowercase).expect("Valid"),
            SeriesCondition::OneShot(BooleanOperator::IsFalse)
        );
        let unknown = json!({"seriesStatus": {"operator": "is", "value": "Hiatus"}});
        assert!(serde_json::from_value::<SeriesCondition>(unknown).is_err());
    }
}
