//! SQL form of series search conditions (Kotlin `SeriesSearchHelper`).

use crate::iden::BookMetadataAggregationAuthorTable as AA;
use crate::iden::BookMetadataAggregationNotableEntitiesTable as AN;
use crate::iden::BookMetadataAggregationTable as A;
use crate::iden::BookMetadataAggregationTagTable as AT;
use crate::iden::ReadProgressSeriesTable as RS;
use crate::iden::SeriesMetadataGenreTable as G;
use crate::iden::SeriesMetadataSharingTable as L;
use crate::iden::SeriesMetadataTable as SD;
use crate::iden::SeriesMetadataTagTable as ST;
use crate::iden::SeriesTable as S;
use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{
    AuthorMatch, BooleanOperator, ContentScope, Equality, NotableEntityMatch, ReadStatus,
    SeriesCondition, TitleField,
};
use sea_query::{
    Alias, Cond, Condition, Expr, ExprTrait, Iden, IntoCondition, JoinType, Query, SelectStatement,
    UnionType,
};

use crate::sqlite::{collation, regexp};
use crate::support::content_restrictions;
use crate::support::search_operator::{self, unicode_1};

/// The joins and condition selecting the series of a search within a user's scope.
pub(super) struct SeriesFilter {
    /// `None` matches every series.
    condition: Option<Condition>,
    /// The library every match belongs to, when the condition or the scope leaves exactly one.
    library: Option<String>,
    /// Collections that `collectionId is` conditions join, each under the alias `CS_<index>`.
    collections: Vec<String>,
}

impl SeriesFilter {
    pub(super) fn new(condition: Option<&SeriesCondition>, scope: &ContentScope) -> Self {
        let collections: Vec<String> = condition
            .map(|condition| {
                condition
                    .collections()
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let library =
            collation::sole_library(scope, condition.and_then(SeriesCondition::sole_library))
                .map(str::to_owned);
        let search = condition.and_then(|condition| compile(condition, &collections));
        let libraries = scope.library_ids().map(|ids| {
            Expr::col((S::Table, S::LibraryId))
                .is_in(ids)
                .into_condition()
        });
        let parts: Vec<Condition> = [
            search,
            content_restrictions::condition(scope.restrictions()),
            libraries,
        ]
        .into_iter()
        .flatten()
        .collect();
        let condition =
            (!parts.is_empty()).then(|| parts.into_iter().fold(Cond::all(), Condition::add));
        Self {
            condition,
            library,
            collections,
        }
    }

    /// Joins the collections and filters `query`, which selects from `SERIES`,
    /// `SERIES_METADATA`, `BOOK_METADATA_AGGREGATION` and the user's `READ_PROGRESS_SERIES`.
    pub(super) fn library(&self) -> Option<&str> {
        self.library.as_deref()
    }

    pub(super) fn apply(&self, query: &mut SelectStatement) {
        for (index, id) in self.collections.iter().enumerate() {
            let alias = collection_alias(index);
            query.join_as(
                JoinType::LeftJoin,
                T::CollectionSeries,
                alias.clone(),
                Expr::col((alias.clone(), K::SeriesId))
                    .equals((S::Table, S::Id))
                    .and(Expr::col((alias, K::CollectionId)).eq(id.as_str())),
            );
        }
        if let Some(condition) = &self.condition {
            query.cond_where(condition.clone());
        }
    }

    /// The number of series in the first joined collection.
    pub(super) fn collection_number(&self) -> Option<Expr> {
        (!self.collections.is_empty()).then(|| Expr::col((collection_alias(0), K::Number)))
    }
}

fn collection_alias(index: usize) -> Alias {
    Alias::new(format!("CS_{index}"))
}

/// `None` matches every series and is left out of `allOf` and `anyOf` alike, like jOOQ's
/// `noCondition`.
fn compile(condition: &SeriesCondition, collections: &[String]) -> Option<Condition> {
    let series = |column| Expr::col((S::Table, column));
    let metadata = |column| Expr::col((SD::Table, column));
    let expression = match condition {
        SeriesCondition::AllOf(conditions) => {
            return combine(conditions, collections, Cond::all());
        }
        SeriesCondition::AnyOf(conditions) => {
            return combine(conditions, collections, Cond::any());
        }
        SeriesCondition::LibraryId(operator) => {
            search_operator::equality(operator, series(S::LibraryId), String::as_str)
        }
        SeriesCondition::CollectionId(Equality::Is { value }) => {
            let index = collections
                .iter()
                .position(|id| id == value)
                .expect("Every collection of the condition is joined");
            Expr::col((collection_alias(index), K::CollectionId)).eq(value.as_str())
        }
        SeriesCondition::CollectionId(Equality::IsNot { value }) => series(S::Id).not_in_subquery(
            Query::select()
                .column(K::SeriesId)
                .from(T::CollectionSeries)
                .and_where(Expr::col(K::CollectionId).eq(value.as_str()))
                .take(),
        ),
        SeriesCondition::Deleted(BooleanOperator::IsTrue) => series(S::DeletedDate).is_not_null(),
        SeriesCondition::Deleted(BooleanOperator::IsFalse) => series(S::DeletedDate).is_null(),
        // A null total book count compares as unknown, so neither operator matches it.
        SeriesCondition::Complete(operator) => {
            let total = metadata(SD::TotalBookCount);
            let count = Expr::col((S::Table, S::BookCount));
            match operator {
                BooleanOperator::IsTrue => total.eq(count),
                BooleanOperator::IsFalse => total.ne(count),
            }
        }
        SeriesCondition::OneShot(operator) => {
            search_operator::boolean(*operator, series(S::Oneshot))
        }
        SeriesCondition::Title(operator) => search_operator::string(operator, metadata(SD::Title)),
        SeriesCondition::TitleSort(operator) => {
            search_operator::string(operator, metadata(SD::TitleSort))
        }
        SeriesCondition::ReleaseDate(operator) => {
            search_operator::date(operator, Expr::col((A::Table, A::ReleaseDate)))
        }
        SeriesCondition::Tag(operator) => {
            search_operator::nullable_membership(operator, series(S::Id), |tag| {
                let mut tagged = series_values(ST::Table, ST::SeriesId, ST::Tag, tag);
                tagged.union(
                    UnionType::Distinct,
                    series_values(AT::Table, AT::SeriesId, AT::Tag, tag),
                );
                tagged
            })
        }
        SeriesCondition::SharingLabel(operator) => {
            search_operator::nullable_membership(operator, series(S::Id), |label| {
                series_values(L::Table, L::SeriesId, L::Label, label)
            })
        }
        SeriesCondition::Genre(operator) => {
            search_operator::nullable_membership(operator, series(S::Id), |genre| {
                series_values(G::Table, G::SeriesId, G::Genre, genre)
            })
        }
        SeriesCondition::Publisher(operator) => {
            search_operator::equality(operator, unicode_1(metadata(SD::Publisher)), String::as_str)
        }
        SeriesCondition::Language(operator) => {
            search_operator::equality(operator, unicode_1(metadata(SD::Language)), String::as_str)
        }
        SeriesCondition::AgeRating(operator) => {
            search_operator::numeric_nullable(operator, metadata(SD::AgeRating))
        }
        SeriesCondition::ReadStatus(operator) => read_status(operator),
        SeriesCondition::SeriesStatus(operator) => {
            search_operator::equality(operator, metadata(SD::Status), |status| status.as_str())
        }
        SeriesCondition::Author(operator) => {
            search_operator::membership(operator, series(S::Id), author)
        }
        SeriesCondition::NotableEntity(operator) => {
            search_operator::membership(operator, series(S::Id), notable_entity)
        }
        SeriesCondition::TitleRegex(regex) => {
            let column = match regex.field() {
                TitleField::Title => SD::Title,
                TitleField::TitleSort => SD::TitleSort,
            };
            regexp::call_sql(regex, metadata(column))
        }
    };
    Some(expression.into_condition())
}

fn combine(
    conditions: &[SeriesCondition],
    collections: &[String],
    empty: Condition,
) -> Option<Condition> {
    let mut compiled = conditions
        .iter()
        .filter_map(|condition| compile(condition, collections))
        .peekable();
    compiled.peek()?;
    Some(compiled.fold(empty, Condition::add))
}

/// The series of `table` whose `column` equals `value` like `unicode1()`, or has any value.
fn series_values<I: Iden + 'static>(
    table: I,
    series: I,
    column: I,
    value: Option<&str>,
) -> SelectStatement {
    let mut query = Query::select();
    query.column(series).from(table);
    match value {
        Some(value) => query.and_where(unicode_1(Expr::col(column)).eq(value)),
        None => query.and_where(Expr::col(column).is_not_null()),
    };
    query
}

/// By the books the user completed: none (without progress) is unread, all is read, and any
/// other count is in progress.
fn read_status(operator: &Equality<ReadStatus>) -> Expr {
    let read = || Expr::col((RS::Table, RS::ReadCount));
    let count = || Expr::col((S::Table, S::BookCount));
    match operator {
        Equality::Is { value } => match value {
            ReadStatus::Unread => read().is_null(),
            ReadStatus::Read => read().eq(count()),
            ReadStatus::InProgress => read().ne(count()),
        },
        Equality::IsNot { value } => match value {
            ReadStatus::Unread => read().is_not_null(),
            ReadStatus::Read => read().ne(count()).or(read().is_null()),
            ReadStatus::InProgress => read().eq(count()).or(read().is_null()),
        },
    }
}

fn author(author: &AuthorMatch) -> SelectStatement {
    let mut query = Query::select();
    query.column(AA::SeriesId).from(AA::Table);
    if let Some(name) = &author.name {
        query.and_where(unicode_1(Expr::col(AA::Name)).eq(name.as_str()));
    }
    if let Some(role) = &author.role {
        query.and_where(unicode_1(Expr::col(AA::Role)).eq(role.as_str()));
    }
    query
}

fn notable_entity(entity: &NotableEntityMatch) -> SelectStatement {
    let mut query = Query::select();
    query.column(AN::SeriesId).from(AN::Table);
    if let Some(name) = &entity.name {
        query.and_where(unicode_1(Expr::col(AN::Name)).eq(name.as_str()));
    }
    if let Some(kind) = &entity.kind {
        query.and_where(unicode_1(Expr::col(AN::Kind)).eq(kind.as_str()));
    }
    query
}
