//! Referential queries over series and book metadata values (Kotlin `ReferentialDao`).

mod query;

use std::slice;

use crate::iden::{
    BookMetadataAggregationTable, BookMetadataAuthorTable, BookMetadataNotableEntitiesTable,
    BookMetadataTagTable, SeriesAndBookTagView, SeriesMetadataGenreTable,
    SeriesMetadataSharingTable, SeriesMetadataTable, SeriesMetadataTagTable,
};
use komga_types::{ContentScope, FilterBy, FilterTags, PageRequest, PageResult};
use sea_query::{DynIden, Expr, ExprTrait, IntoIden, Order, SelectStatement};

use self::query::{Owner, contains, fetch_page, single_column, values_query};
use crate::connection::Database;
use crate::error::DbError;
use crate::sqlite::collation;

/// DAO for referential metadata values.
pub struct ReferentialDao;

/// A text value held per series, listed by [`ReferentialDao::find_series_values`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SeriesValue {
    Genre,
    SharingLabel,
    Language,
    Publisher,
}

/// A named, kinded credit on books, listed by [`ReferentialDao::find_credits`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Credit {
    /// `BOOK_METADATA_AUTHOR`, whose kind is the role.
    Author,
    /// `BOOK_METADATA_NOTABLE_ENTITIES`.
    NotableEntity,
}

impl Credit {
    fn table(self) -> DynIden {
        match self {
            Self::Author => BookMetadataAuthorTable::Table.into_iden(),
            Self::NotableEntity => BookMetadataNotableEntitiesTable::Table.into_iden(),
        }
    }

    fn name(self) -> Expr {
        match self {
            Self::Author => Expr::col((
                BookMetadataAuthorTable::Table,
                BookMetadataAuthorTable::Name,
            )),
            Self::NotableEntity => Expr::col((
                BookMetadataNotableEntitiesTable::Table,
                BookMetadataNotableEntitiesTable::Name,
            )),
        }
    }

    fn kind(self) -> Expr {
        match self {
            Self::Author => Expr::col((
                BookMetadataAuthorTable::Table,
                BookMetadataAuthorTable::Role,
            )),
            Self::NotableEntity => Expr::col((
                BookMetadataNotableEntitiesTable::Table,
                BookMetadataNotableEntitiesTable::Kind,
            )),
        }
    }
}

impl ReferentialDao {
    /// Distinct non-blank values of `kind` on series visible in `scope`, those containing
    /// `search` ignoring accents and ASCII case, in Unicode order.
    pub async fn find_series_values(
        database: &Database,
        scope: &ContentScope,
        kind: SeriesValue,
        search: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let (table, column): (DynIden, DynIden) = match kind {
            SeriesValue::Genre => (
                SeriesMetadataGenreTable::Table.into_iden(),
                SeriesMetadataGenreTable::Genre.into_iden(),
            ),
            SeriesValue::SharingLabel => (
                SeriesMetadataSharingTable::Table.into_iden(),
                SeriesMetadataSharingTable::Label.into_iden(),
            ),
            SeriesValue::Language => (
                SeriesMetadataTable::Table.into_iden(),
                SeriesMetadataTable::Language.into_iden(),
            ),
            SeriesValue::Publisher => (
                SeriesMetadataTable::Table.into_iden(),
                SeriesMetadataTable::Publisher.into_iden(),
            ),
        };
        let value = Expr::col((table.clone(), column));
        let mut query = values_query(table, Owner::Series, slice::from_ref(&value), scope, filter);
        // Language and publisher are blank when unset.
        query.and_where(value.clone().ne(""));
        if let Some(search) = search {
            query.and_where(contains(value.clone(), search));
        }
        let order = [(
            collation::collate(value, &names(database, scope, filter)),
            Order::Asc,
        )];
        Ok(single_column(
            fetch_page(database, query, &order, page).await?,
        ))
    }

    /// Distinct series age ratings visible in `scope`, ascending.
    pub async fn find_age_ratings(
        database: &Database,
        scope: &ContentScope,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<i32>, DbError> {
        let value = Expr::col((SeriesMetadataTable::Table, SeriesMetadataTable::AgeRating));
        let mut query = age_ratings_query(&value, scope, filter);
        query.and_where(value.clone().is_not_null());
        Ok(single_column(
            fetch_page(database, query, &[(value, Order::Asc)], page).await?,
        ))
    }

    /// Like [`Self::find_age_ratings`], unpaged, with `None` first when a series is unrated.
    pub async fn find_age_ratings_with_unrated(
        database: &Database,
        scope: &ContentScope,
        filter: Option<&FilterBy>,
    ) -> Result<Vec<Option<i32>>, DbError> {
        let value = Expr::col((SeriesMetadataTable::Table, SeriesMetadataTable::AgeRating));
        let query = age_ratings_query(&value, scope, filter);
        let order = [(value, Order::Asc)];
        let ratings = fetch_page(database, query, &order, PageRequest::Unpaged).await?;
        Ok(single_column(ratings).items)
    }

    /// Distinct years of the series release dates visible in `scope`, latest first.
    pub async fn find_series_release_years(
        database: &Database,
        scope: &ContentScope,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let release_date = Expr::col((
            BookMetadataAggregationTable::Table,
            BookMetadataAggregationTable::ReleaseDate,
        ));
        let year = Expr::cust_with_expr("CAST(strftime('%Y', ?) AS INTEGER)", release_date);
        let mut query = values_query(
            BookMetadataAggregationTable::Table.into_iden(),
            Owner::Series,
            slice::from_ref(&year),
            scope,
            filter,
        );
        query.and_where(year.clone().is_not_null());
        let order = [(year, Order::Desc)];
        let years: PageResult<i64> =
            single_column(fetch_page(database, query, &order, page).await?);
        Ok(PageResult {
            items: years.items.iter().map(i64::to_string).collect(),
            total_elements: years.total_elements,
        })
    }

    /// Distinct `(name, kind)` pairs of the `credit` on books visible in `scope` whose name
    /// contains `search` and whose kind is `kind`, by name then kind.
    pub async fn find_credits(
        database: &Database,
        scope: &ContentScope,
        credit: Credit,
        search: Option<&str>,
        kind: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<(String, String)>, DbError> {
        let columns = [credit.name(), credit.kind()];
        let query = credits_query(credit, &columns, scope, search, kind, filter);
        let order = [
            (
                collation::collate(credit.name(), &names(database, scope, filter)),
                Order::Asc,
            ),
            (credit.kind(), Order::Asc),
        ];
        fetch_page(database, query, &order, page).await
    }

    /// Distinct names of the pairs [`Self::find_credits`] lists, in Unicode order.
    pub async fn find_credit_names(
        database: &Database,
        scope: &ContentScope,
        credit: Credit,
        search: Option<&str>,
        kind: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let name = credit.name();
        let query = credits_query(credit, slice::from_ref(&name), scope, search, kind, filter);
        let order = [(
            collation::collate(name, &names(database, scope, filter)),
            Order::Asc,
        )];
        Ok(single_column(
            fetch_page(database, query, &order, page).await?,
        ))
    }

    /// Distinct kinds of the `credit` on books visible in `scope`, in code point order.
    pub async fn find_credit_kinds(
        database: &Database,
        scope: &ContentScope,
        credit: Credit,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let kind = credit.kind();
        let query = credits_query(credit, slice::from_ref(&kind), scope, None, None, filter);
        Ok(single_column(
            fetch_page(database, query, &[(kind, Order::Asc)], page).await?,
        ))
    }

    /// Distinct tags of the `source` visible in `scope` that contain `search`, in Unicode order.
    /// Book tags of [`FilterTags::Both`] come from the series aggregation.
    pub async fn find_tags(
        database: &Database,
        scope: &ContentScope,
        search: Option<&str>,
        source: FilterTags,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let (table, column, owner): (DynIden, DynIden, Owner) = match source {
            FilterTags::Series => (
                SeriesMetadataTagTable::Table.into_iden(),
                SeriesMetadataTagTable::Tag.into_iden(),
                Owner::Series,
            ),
            FilterTags::Book => (
                BookMetadataTagTable::Table.into_iden(),
                BookMetadataTagTable::Tag.into_iden(),
                Owner::Book,
            ),
            FilterTags::Both => (
                SeriesAndBookTagView::Table.into_iden(),
                SeriesAndBookTagView::Tag.into_iden(),
                Owner::Series,
            ),
        };
        let tag = Expr::col((table.clone(), column));
        let mut query = values_query(table, owner, slice::from_ref(&tag), scope, filter);
        if let Some(search) = search {
            query.and_where(contains(tag.clone(), search));
        }
        let order = [(
            collation::collate(tag, &names(database, scope, filter)),
            Order::Asc,
        )];
        Ok(single_column(
            fetch_page(database, query, &order, page).await?,
        ))
    }
}

fn age_ratings_query(
    value: &Expr,
    scope: &ContentScope,
    filter: Option<&FilterBy>,
) -> SelectStatement {
    let table = SeriesMetadataTable::Table.into_iden();
    values_query(table, Owner::Series, slice::from_ref(value), scope, filter)
}

/// Rows of `columns` of the `credit` table, the name containing `search` and the kind `kind`.
fn credits_query(
    credit: Credit,
    columns: &[Expr],
    scope: &ContentScope,
    search: Option<&str>,
    kind: Option<&str>,
    filter: Option<&FilterBy>,
) -> SelectStatement {
    let mut query = values_query(credit.table(), Owner::Book, columns, scope, filter);
    if let Some(search) = search {
        query.and_where(contains(credit.name(), search));
    }
    if let Some(kind) = kind {
        query.and_where(credit.kind().eq(kind));
    }
    query
}

/// The collation ordering names listed within `scope` and `filter`. Only a library filter narrows
/// the listing to one library: the other filters are treated as spanning libraries.
fn names(database: &Database, scope: &ContentScope, filter: Option<&FilterBy>) -> String {
    let library = match filter {
        Some(FilterBy::Libraries(libraries)) if libraries.len() == 1 => {
            libraries.first().map(|library| library.as_str())
        }
        _ => None,
    };
    database
        .sort_rules()
        .collation(collation::sole_library(scope, library))
}
