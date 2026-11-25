//! Kotlin `SearchCondition.Book`: filters of a book list query.

use std::str::FromStr;

use serde::Deserialize;

use crate::domain::{
    BooleanOperator, DateOperator, Equality, EqualityNullable, Numeric, StringOperator,
};
use crate::{MediaProfile, MediaStatus, ThumbnailType};

/// A book condition; its JSON form is an object whose single property names the condition.
#[derive(Debug, Clone, PartialEq, Deserialize)]
#[serde(rename_all = "camelCase")]
pub enum BookCondition {
    AnyOf(Vec<BookCondition>),
    AllOf(Vec<BookCondition>),
    LibraryId(Equality<String>),
    ReadListId(Equality<String>),
    SeriesId(Equality<String>),
    Deleted(BooleanOperator),
    OneShot(BooleanOperator),
    Title(StringOperator),
    ReleaseDate(DateOperator),
    Tag(EqualityNullable<String>),
    NumberSort(Numeric<f32>),
    ReadStatus(Equality<ReadStatus>),
    MediaStatus(Equality<MediaStatus>),
    MediaProfile(Equality<MediaProfile>),
    Author(Equality<AuthorMatch>),
    NotableEntity(Equality<NotableEntityMatch>),
    Poster(Equality<PosterMatch>),
}

impl BookCondition {
    /// Read lists the books must belong to, in the order they appear; the first orders books by
    /// their number in it.
    #[must_use]
    pub fn read_lists(&self) -> Vec<&str> {
        let mut ids = Vec::new();
        self.collect_read_lists(&mut ids);
        ids
    }

    /// The library every matching book belongs to, when the condition names exactly one.
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

    fn collect_read_lists<'condition>(&'condition self, ids: &mut Vec<&'condition str>) {
        match self {
            Self::AnyOf(conditions) | Self::AllOf(conditions) => {
                for condition in conditions {
                    condition.collect_read_lists(ids);
                }
            }
            Self::ReadListId(Equality::Is { value }) if !ids.contains(&value.as_str()) => {
                ids.push(value);
            }
            _ => {}
        }
    }
}

/// Kotlin `ReadStatus` of a book for one user.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Deserialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ReadStatus {
    Unread,
    Read,
    InProgress,
}

impl FromStr for ReadStatus {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "UNREAD" => Ok(Self::Unread),
            "READ" => Ok(Self::Read),
            "IN_PROGRESS" => Ok(Self::InProgress),
            other => Err(format!("Unknown read status {other}")),
        }
    }
}

/// `SearchCondition.AuthorMatch`; an absent field matches any value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct AuthorMatch {
    pub name: Option<String>,
    pub role: Option<String>,
}

/// `SearchCondition.NotableEntityMatch`; an absent field matches any value.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct NotableEntityMatch {
    pub name: Option<String>,
    pub kind: Option<String>,
}

/// `SearchCondition.PosterMatch`; with neither field the condition matches every book.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize)]
pub struct PosterMatch {
    #[serde(rename = "type")]
    pub kind: Option<ThumbnailType>,
    pub selected: Option<bool>,
}

/// `BookDtoDao.sorts`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BookSort {
    Name,
    SeriesTitleSort,
    CreatedDate,
    LastModifiedDate,
    FileSize,
    FileHash,
    Url,
    MediaStatus,
    MediaComment,
    MediaType,
    MediaPageCount,
    Title,
    NumberSort,
    ReleaseDate,
    ReadProgressLastModified,
    ReadDate,
    /// Only applies when the condition selects a read list.
    ReadListNumber,
    /// Only applies with a full-text search: its most relevant hits first.
    Relevance,
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::BookCondition;
    use crate::domain::{BooleanOperator, Equality};

    #[test]
    fn conditions_are_named_by_their_single_property() {
        let condition: BookCondition = serde_json::from_value(json!({"allOf": [
            {"oneShot": {"operator": "isFalse"}},
            {"anyOf": [
                {"readListId": {"operator": "is", "value": "b"}},
                {"readListId": {"operator": "isNot", "value": "c"}},
                {"readListId": {"operator": "is", "value": "a"}},
                {"readListId": {"operator": "is", "value": "b"}},
            ]},
        ]}))
        .expect("Valid");
        let BookCondition::AllOf(conditions) = &condition else {
            panic!("Parsed {condition:?}");
        };
        assert_eq!(
            conditions[0],
            BookCondition::OneShot(BooleanOperator::IsFalse)
        );
        assert_eq!(condition.read_lists(), ["b", "a"]);

        let two = json!({"seriesId": {"operator": "is", "value": "a"}, "deleted": {"operator": "isTrue"}});
        assert!(serde_json::from_value::<BookCondition>(two).is_err());
        let wrong_case = json!({"SeriesId": {"operator": "is", "value": "a"}});
        assert!(serde_json::from_value::<BookCondition>(wrong_case).is_err());
        let series: BookCondition =
            serde_json::from_value(json!({"seriesId": {"operator": "is", "value": "a"}}))
                .expect("Valid");
        assert_eq!(
            series,
            BookCondition::SeriesId(Equality::Is { value: "a".into() })
        );
    }
}
