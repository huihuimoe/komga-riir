//! Kotlin `PageHashDao`.

use crate::iden::BookTable as B;
use crate::iden::MediaPageTable as P;
use crate::iden::PageHashTable as PH;
use crate::iden::PageHashThumbnailTable as PHT;
use komga_types::{
    BookId, BookPageNumbered, KnownPageHashSort, LibraryId, PageHashAction, PageHashKnown,
    PageHashMatch, PageHashMatchSort, PageHashUnknown, PageRequest, PageResult, SortOrder,
    UnknownPageHashSort,
};
use sea_query::{Alias, Expr, ExprTrait, Func, Order, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::sqlite::SqliteRow;
use sqlx::{AssertSqlSafe, Row};

use crate::support::local_date_time::{self, SqlDateTime};
use crate::support::rows::fetch_page;
use crate::{Database, DbError, DbTransaction};

pub struct PageHashDao;

impl PageHashDao {
    /// Whether a known hash exists.
    pub async fn exists_known(
        transaction: &mut DbTransaction,
        hash: &str,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::select()
            .expr(Expr::val(1))
            .from(PH::Table)
            .and_where(Expr::col(PH::Hash).eq(hash))
            .build_sqlx(SqliteQueryBuilder);
        let row = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(row.is_some())
    }

    /// Known hashes with the number of pages having each, optionally only those with one of
    /// `actions`.
    pub async fn find_all_known(
        database: &Database,
        actions: Option<&[PageHashAction]>,
        sort: &[SortOrder<KnownPageHashSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashKnown>, DbError> {
        let columns = [
            PH::Hash,
            PH::Size,
            PH::Action,
            PH::DeleteCount,
            PH::CreatedDate,
            PH::LastModifiedDate,
        ];
        let mut query = Query::select();
        query
            .columns(columns.map(|column| (PH::Table, column)))
            .expr_as(
                Func::count(Expr::col((P::Table, P::FileHash))),
                Alias::new("count"),
            )
            .from(PH::Table)
            .left_join(
                P::Table,
                Expr::col((PH::Table, PH::Hash)).equals((P::Table, P::FileHash)),
            );
        if let Some(actions) = actions {
            query.and_where(Expr::col((PH::Table, PH::Action)).is_in(actions.iter().copied()));
        }
        query.group_by_columns(columns.map(|column| (PH::Table, column)));
        let orders = sort
            .iter()
            .map(|order| (known_sort_expression(order.key), order.direction));
        fetch_page(database, query, orders, page, known_from_row).await
    }

    /// Hashes without an action that more than one page has.
    pub async fn find_all_unknown(
        database: &Database,
        sort: &[SortOrder<UnknownPageHashSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashUnknown>, DbError> {
        let book_count = || Func::count(Expr::col((P::Table, P::BookId)));
        let mut known = Query::select();
        known
            .expr(Expr::val(1))
            .from(PH::Table)
            .and_where(Expr::col((PH::Table, PH::Hash)).equals((P::Table, P::FileHash)));
        let mut query = Query::select();
        query
            .column((P::Table, P::FileHash))
            .column((P::Table, P::FileSize))
            .expr_as(book_count(), Alias::new("count"))
            .expr_as(
                book_count().mul(Expr::col((P::Table, P::FileSize))),
                Alias::new("totalSize"),
            )
            .from(P::Table)
            .and_where(Expr::col((P::Table, P::FileHash)).ne(""))
            .and_where(Expr::exists(known).not())
            .group_by_col((P::Table, P::FileHash))
            .and_having(book_count().gt(1));
        let orders = sort
            .iter()
            .map(|order| (unknown_sort_expression(order.key), order.direction));
        fetch_page(database, query, orders, page, |row| {
            Ok(PageHashUnknown {
                hash: row.try_get("FILE_HASH")?,
                size: page_hash_size(row.try_get("FILE_SIZE")?),
                match_count: row.try_get("count")?,
            })
        })
        .await
    }

    /// The pages with `hash`, deleted books included.
    pub async fn find_matches_by_hash(
        database: &Database,
        hash: &str,
        sort: &[SortOrder<PageHashMatchSort>],
        page: PageRequest,
    ) -> Result<PageResult<PageHashMatch>, DbError> {
        let mut query = Query::select();
        query
            .columns([
                (P::Table, P::BookId),
                (P::Table, P::Number),
                (P::Table, P::FileName),
                (P::Table, P::FileSize),
                (P::Table, P::MediaType),
            ])
            .column((B::Table, B::Url))
            .from(P::Table)
            .left_join(
                B::Table,
                Expr::col((P::Table, P::BookId)).equals((B::Table, B::Id)),
            )
            .and_where(Expr::col((P::Table, P::FileHash)).eq(hash));
        let orders = sort
            .iter()
            .map(|order| (match_sort_expression(order.key), order.direction));
        fetch_page(database, query, orders, page, |row| {
            let number: i32 = row.try_get("NUMBER")?;
            Ok(PageHashMatch {
                book_id: row.try_get("BOOK_ID")?,
                url: row.try_get("URL")?,
                page_number: number + 1,
                file_name: row.try_get("FILE_NAME")?,
                file_size: row.try_get("FILE_SIZE")?,
                media_type: row.try_get("MEDIA_TYPE")?,
            })
        })
        .await
    }

    /// The thumbnail stored when the hash became known.
    pub async fn find_known_thumbnail(
        database: &Database,
        hash: &str,
    ) -> Result<Option<Vec<u8>>, DbError> {
        let (sql, values) = Query::select()
            .column(PHT::Thumbnail)
            .from(PHT::Table)
            .and_where(Expr::col(PHT::Hash).eq(hash))
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(database.read_pool())
            .await?)
    }

    /// Kotlin `insert`: dates and the delete count take their defaults.
    pub async fn insert(
        transaction: &mut DbTransaction,
        hash: &str,
        size: Option<i64>,
        action: PageHashAction,
        thumbnail: Option<&[u8]>,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(PH::Table)
            .columns([PH::Hash, PH::Size, PH::Action])
            .values_panic([hash.into(), page_hash_size(size).into(), action.into()])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        if let Some(thumbnail) = thumbnail {
            let (sql, values) = Query::insert()
                .into_table(PHT::Table)
                .columns([PHT::Hash, PHT::Thumbnail])
                .values_panic([hash.into(), thumbnail.to_vec().into()])
                .build_sqlx(SqliteQueryBuilder);
            sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
                .execute(transaction.connection())
                .await?;
        }
        Ok(())
    }

    /// Kotlin `update` of a stored hash copied with a new action: the size and delete count
    /// are written back unchanged.
    pub async fn update_action(
        transaction: &mut DbTransaction,
        hash: &str,
        action: PageHashAction,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(PH::Table)
            .value(PH::Action, action)
            .value(PH::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(PH::Hash).eq(hash))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Kotlin `findMatchesByKnownHashAction`: the pages of the library's books whose hash is known
    /// with `action`, in page order.
    pub async fn find_matches_by_known_action(
        transaction: &mut DbTransaction,
        action: PageHashAction,
        library_id: &LibraryId,
    ) -> Result<Vec<(BookId, BookPageNumbered)>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                (P::Table, P::BookId),
                (P::Table, P::Number),
                (P::Table, P::FileName),
                (P::Table, P::FileHash),
                (P::Table, P::FileSize),
                (P::Table, P::MediaType),
            ])
            .from(P::Table)
            .inner_join(
                PH::Table,
                Expr::col((P::Table, P::FileHash)).equals((PH::Table, PH::Hash)),
            )
            .inner_join(
                B::Table,
                Expr::col((P::Table, P::BookId)).equals((B::Table, B::Id)),
            )
            .and_where(Expr::col((PH::Table, PH::Action)).eq(action))
            .and_where(Expr::col((B::Table, B::LibraryId)).eq(library_id.as_str()))
            .order_by((P::Table, P::BookId), Order::Asc)
            .order_by((P::Table, P::Number), Order::Asc)
            .build_sqlx(SqliteQueryBuilder);
        let rows = sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.iter()
            .map(|row| {
                let number: i32 = row.try_get("NUMBER")?;
                let page = BookPageNumbered {
                    file_name: row.try_get("FILE_NAME")?,
                    media_type: row.try_get("MEDIA_TYPE")?,
                    dimension: None,
                    file_hash: row.try_get("FILE_HASH")?,
                    file_size: row.try_get("FILE_SIZE")?,
                    page_number: number + 1,
                };
                Ok((row.try_get("BOOK_ID")?, page))
            })
            .collect()
    }

    /// Counts a deleted page against its known hash, if any.
    pub async fn count_deletion(
        transaction: &mut DbTransaction,
        hash: &str,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(PH::Table)
            .value(PH::DeleteCount, Expr::col(PH::DeleteCount).add(1))
            .value(PH::LastModifiedDate, local_date_time::now_utc())
            .and_where(Expr::col(PH::Hash).eq(hash))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }
}

/// `count` is the select alias.
fn known_sort_expression(key: KnownPageHashSort) -> Expr {
    match key {
        KnownPageHashSort::Hash => Expr::col((PH::Table, PH::Hash)),
        KnownPageHashSort::MatchCount => Expr::col(Alias::new("count")),
        KnownPageHashSort::DeleteCount => Expr::col((PH::Table, PH::DeleteCount)),
        KnownPageHashSort::DeleteSize => {
            Expr::col((PH::Table, PH::Size)).mul(Expr::col((PH::Table, PH::DeleteCount)))
        }
        KnownPageHashSort::Size => Expr::col((PH::Table, PH::Size)),
        KnownPageHashSort::Created => Expr::col((PH::Table, PH::CreatedDate)),
        KnownPageHashSort::LastModified => Expr::col((PH::Table, PH::LastModifiedDate)),
    }
}

/// `count` and `totalSize` are the select aliases.
fn unknown_sort_expression(key: UnknownPageHashSort) -> Expr {
    match key {
        UnknownPageHashSort::Hash => Expr::col((P::Table, P::FileHash)),
        UnknownPageHashSort::Size => Expr::col((P::Table, P::FileSize)),
        UnknownPageHashSort::MatchCount => Expr::col(Alias::new("count")),
        UnknownPageHashSort::TotalSize => Expr::col(Alias::new("totalSize")),
    }
}

fn match_sort_expression(key: PageHashMatchSort) -> Expr {
    match key {
        PageHashMatchSort::Size => Expr::col((P::Table, P::FileSize)),
        PageHashMatchSort::Url => Expr::col((B::Table, B::Url)),
        PageHashMatchSort::BookId => Expr::col((P::Table, P::BookId)),
        PageHashMatchSort::PageNumber => Expr::col((P::Table, P::Number)),
    }
}

/// Kotlin `PageHash.size`: a negative size is unknown.
fn page_hash_size(size: Option<i64>) -> Option<i64> {
    size.filter(|size| *size >= 0)
}

fn known_from_row(row: &SqliteRow) -> Result<PageHashKnown, DbError> {
    Ok(PageHashKnown {
        hash: row.try_get("HASH")?,
        size: page_hash_size(row.try_get("SIZE")?),
        action: row.try_get("ACTION")?,
        delete_count: row.try_get("DELETE_COUNT")?,
        match_count: row.try_get("count")?,
        created_date: row.try_get::<SqlDateTime, _>("CREATED_DATE")?.0,
        last_modified_date: row.try_get::<SqlDateTime, _>("LAST_MODIFIED_DATE")?.0,
    })
}
