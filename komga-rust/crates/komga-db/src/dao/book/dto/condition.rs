//! SQL form of book search conditions (Kotlin `BookSearchHelper`).

use crate::iden::BookMetadataAuthorTable as A;
use crate::iden::BookMetadataNotableEntitiesTable as NE;
use crate::iden::BookMetadataTable as D;
use crate::iden::BookMetadataTagTable as BT;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressTable as R;
use crate::iden::ThumbnailBookTable as TB;
use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{
    AuthorMatch, BookCondition, BooleanOperator, ContentScope, Equality, EqualityNullable,
    NotableEntityMatch, PosterMatch, ReadStatus,
};
use sea_query::{
    Alias, Cond, Condition, Expr, ExprTrait, Func, IntoCondition, JoinType, Query, SelectStatement,
};

use crate::sqlite::collation;
use crate::support::content_restrictions;
use crate::support::search_operator::{self, unicode_1};

/// The joins and condition selecting the books of a search within a user's scope.
pub(super) struct BookFilter {
    /// `None` matches every book.
    condition: Option<Condition>,
    /// The library every match belongs to, when the condition or the scope leaves exactly one.
    library: Option<String>,
    /// Read lists that `readListId is` conditions join, each under the alias `RLB_<index>`.
    read_lists: Vec<String>,
}

impl BookFilter {
    pub(super) fn new(condition: Option<&BookCondition>, scope: &ContentScope) -> Self {
        let read_lists: Vec<String> = condition
            .map(|condition| {
                condition
                    .read_lists()
                    .into_iter()
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default();
        let library =
            collation::sole_library(scope, condition.and_then(BookCondition::sole_library))
                .map(str::to_owned);
        let search = condition.and_then(|condition| compile(condition, &read_lists));
        let libraries = scope.library_ids().map(|ids| {
            Expr::col((B::Table, B::LibraryId))
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
            read_lists,
        }
    }

    /// Joins the read lists and filters `query`, which selects from `BOOK`, `MEDIA`,
    /// `BOOK_METADATA`, `SERIES_METADATA` and the user's `READ_PROGRESS`.
    pub(super) fn library(&self) -> Option<&str> {
        self.library.as_deref()
    }

    pub(super) fn apply(&self, query: &mut SelectStatement) {
        for (index, id) in self.read_lists.iter().enumerate() {
            let alias = read_list_alias(index);
            query.join_as(
                JoinType::LeftJoin,
                T::ReadlistBook,
                alias.clone(),
                Expr::col((alias.clone(), K::BookId))
                    .equals((B::Table, B::Id))
                    .and(Expr::col((alias, K::ReadlistId)).eq(id.as_str())),
            );
        }
        if let Some(condition) = &self.condition {
            query.cond_where(condition.clone());
        }
    }

    /// The number of books in the first joined read list.
    pub(super) fn read_list_number(&self) -> Option<Expr> {
        (!self.read_lists.is_empty()).then(|| Expr::col((read_list_alias(0), K::Number)))
    }
}

fn read_list_alias(index: usize) -> Alias {
    Alias::new(format!("RLB_{index}"))
}

/// `None` matches every book and is left out of `allOf` and `anyOf` alike, like jOOQ's
/// `noCondition`.
fn compile(condition: &BookCondition, read_lists: &[String]) -> Option<Condition> {
    let book = |column| Expr::col((B::Table, column));
    let metadata = |column| Expr::col((D::Table, column));
    let expression = match condition {
        BookCondition::AllOf(conditions) => {
            return combine(conditions, read_lists, Cond::all());
        }
        BookCondition::AnyOf(conditions) => {
            return combine(conditions, read_lists, Cond::any());
        }
        BookCondition::LibraryId(operator) => {
            search_operator::equality(operator, book(B::LibraryId), String::as_str)
        }
        BookCondition::SeriesId(operator) => {
            search_operator::equality(operator, book(B::SeriesId), String::as_str)
        }
        BookCondition::ReadListId(Equality::Is { value }) => {
            let index = read_lists
                .iter()
                .position(|id| id == value)
                .expect("Every read list of the condition is joined");
            Expr::col((read_list_alias(index), K::ReadlistId)).eq(value.as_str())
        }
        BookCondition::ReadListId(operator) => {
            search_operator::membership(operator, book(B::Id), |id| {
                in_book_table(T::ReadlistBook, K::BookId, |query| {
                    query.and_where(Expr::col(K::ReadlistId).eq(id.as_str()));
                })
            })
        }
        BookCondition::Deleted(BooleanOperator::IsTrue) => book(B::DeletedDate).is_not_null(),
        BookCondition::Deleted(BooleanOperator::IsFalse) => book(B::DeletedDate).is_null(),
        BookCondition::OneShot(operator) => search_operator::boolean(*operator, book(B::Oneshot)),
        BookCondition::Title(operator) => search_operator::string(operator, metadata(D::Title)),
        BookCondition::ReleaseDate(operator) => {
            search_operator::date(operator, metadata(D::ReleaseDate))
        }
        BookCondition::NumberSort(operator) => {
            search_operator::numeric(operator, metadata(D::NumberSort))
        }
        BookCondition::Tag(operator) => tag(operator),
        BookCondition::ReadStatus(operator) => read_status(operator),
        BookCondition::MediaStatus(operator) => {
            search_operator::equality(operator, Expr::col((M::Table, M::Status)), |status| {
                status.as_str()
            })
        }
        BookCondition::MediaProfile(operator) => {
            let media_type = Expr::col((M::Table, M::MediaType));
            match operator {
                Equality::Is { value } => media_type.is_in(value.media_types().iter().copied()),
                Equality::IsNot { value } => {
                    media_type.is_not_in(value.media_types().iter().copied())
                }
            }
        }
        BookCondition::Author(operator) => {
            search_operator::membership(operator, book(B::Id), author)
        }
        BookCondition::NotableEntity(operator) => {
            search_operator::membership(operator, book(B::Id), notable_entity)
        }
        BookCondition::Poster(operator) => {
            let (Equality::Is { value } | Equality::IsNot { value }) = operator;
            if value.kind.is_none() && value.selected.is_none() {
                return None;
            }
            search_operator::membership(operator, book(B::Id), poster)
        }
    };
    Some(expression.into_condition())
}

fn combine(
    conditions: &[BookCondition],
    read_lists: &[String],
    empty: Condition,
) -> Option<Condition> {
    let mut compiled = conditions
        .iter()
        .filter_map(|condition| compile(condition, read_lists))
        .peekable();
    compiled.peek()?;
    Some(compiled.fold(empty, Condition::add))
}

/// `SELECT <book> FROM <table>` narrowed by `narrow`.
fn in_book_table(
    table: impl sea_query::Iden + 'static,
    book: impl sea_query::Iden + 'static,
    narrow: impl FnOnce(&mut SelectStatement),
) -> SelectStatement {
    let mut query = Query::select();
    query.column(book).from(table);
    narrow(&mut query);
    query
}

fn tag(operator: &EqualityNullable<String>) -> Expr {
    search_operator::nullable_membership(operator, Expr::col((B::Table, B::Id)), |tag| {
        in_book_table(BT::Table, BT::BookId, |query| match tag {
            Some(tag) => {
                query.and_where(unicode_1(Expr::col(BT::Tag)).eq(tag));
            }
            None => {
                query.and_where(Expr::col(BT::Tag).is_not_null());
            }
        })
    })
}

/// Without progress a book is unread.
fn read_status(operator: &Equality<ReadStatus>) -> Expr {
    let completed = || Expr::col((R::Table, R::Completed));
    match operator {
        Equality::Is { value } => match value {
            ReadStatus::Unread => completed().is_null(),
            ReadStatus::Read => completed().eq(true),
            ReadStatus::InProgress => completed().eq(false),
        },
        Equality::IsNot { value } => match value {
            ReadStatus::Unread => completed().is_not_null(),
            ReadStatus::Read => completed().is_null().or(completed().eq(false)),
            ReadStatus::InProgress => completed().eq(true).or(completed().is_null()),
        },
    }
}

fn author(author: &AuthorMatch) -> SelectStatement {
    in_book_table(A::Table, A::BookId, |query| {
        if let Some(name) = &author.name {
            query.and_where(unicode_1(Expr::col(A::Name)).eq(name.as_str()));
        }
        if let Some(role) = &author.role {
            query.and_where(unicode_1(Expr::col(A::Role)).eq(role.as_str()));
        }
    })
}

fn notable_entity(entity: &NotableEntityMatch) -> SelectStatement {
    in_book_table(NE::Table, NE::BookId, |query| {
        if let Some(name) = &entity.name {
            query.and_where(unicode_1(Expr::col(NE::Name)).eq(name.as_str()));
        }
        if let Some(kind) = &entity.kind {
            query.and_where(unicode_1(Expr::col(NE::Kind)).eq(kind.as_str()));
        }
    })
}

/// The type compares case-insensitively.
fn poster(poster: &PosterMatch) -> SelectStatement {
    in_book_table(TB::Table, TB::BookId, |query| {
        if let Some(kind) = poster.kind {
            query.and_where(Func::lower(Expr::col(TB::Type)).eq(kind.name().to_ascii_lowercase()));
        }
        if let Some(selected) = poster.selected {
            query.and_where(Expr::col(TB::Selected).eq(selected));
        }
    })
}
