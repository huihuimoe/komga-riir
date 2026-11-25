use crate::iden::BookMetadataTable as D;
use crate::iden::BookTable as B;
use crate::iden::MediaTable as M;
use crate::iden::ReadProgressTable as R;
use komga_types::{Book, BookId, FileUrl, LibraryId, SeriesId, UserId};
use sea_query::{Expr, ExprTrait, Order, Query, SelectStatement, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{BookDao, BookHash, COLUMNS};
use crate::{DbError, DbTransaction};

impl BookDao {
    /// `findAll` for books of a library whose media has one of `statuses`, ordered by series and
    /// number; soft-deleted books included.
    pub async fn find_all_by_library_and_media_statuses(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        statuses: &[&str],
    ) -> Result<Vec<Book>, DbError> {
        let query = with_media(library_id)
            .and_where(Expr::col((M::Table, M::Status)).is_in(statuses.iter().copied()))
            .order_by((B::Table, B::SeriesId), Order::Asc)
            .order_by((B::Table, B::Number), Order::Asc)
            .take();
        Self::find(transaction, query).await
    }

    pub async fn find_all_by_library_and_media_types(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        media_types: &[&str],
    ) -> Result<Vec<Book>, DbError> {
        let query = with_media(library_id)
            .and_where(Expr::col((M::Table, M::MediaType)).is_in(media_types.iter().copied()))
            .order_by((B::Table, B::SeriesId), Order::Asc)
            .order_by((B::Table, B::Number), Order::Asc)
            .take();
        Self::find(transaction, query).await
    }

    /// `findAllByLibraryIdAndMismatchedExtension`; SQLite `LIKE` ignores ASCII case, so `.CBZ`
    /// matches `cbz`.
    pub async fn find_all_by_library_and_mismatched_extension(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        media_type: &str,
        extension: &str,
    ) -> Result<Vec<Book>, DbError> {
        let query = with_media(library_id)
            .and_where(Expr::col((M::Table, M::MediaType)).eq(media_type))
            .and_where(Expr::col((B::Table, B::Url)).not_like(format!("%.{extension}")))
            .take();
        Self::find(transaction, query).await
    }

    /// `findAllByHashKoreader`, without soft-deleted books.
    pub async fn find_all_by_hash_koreader(
        transaction: &mut DbTransaction,
        hash: &str,
    ) -> Result<Vec<Book>, DbError> {
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::FileHashKoreader).eq(hash))
            .and_where(Expr::col(B::DeletedDate).is_null())
            .take();
        Self::find(transaction, query).await
    }

    /// `findAllByLibraryIdAndWithEmptyHash` and `findAllByLibraryIdAndWithEmptyHashKoreader`.
    pub async fn find_all_by_library_with_empty_hash(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        hash: BookHash,
    ) -> Result<Vec<Book>, DbError> {
        let column = match hash {
            BookHash::File => B::FileHash,
            BookHash::Koreader => B::FileHashKoreader,
        };
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::LibraryId).eq(library_id))
            .and_where(Expr::col(column).eq(""))
            .take();
        Self::find(transaction, query).await
    }

    /// `findAllDeletedByFileSize` for any of `sizes`, keeping hashed books: the candidates of
    /// `tryRestoreSeries` and `tryRestoreBooks`, from every library.
    pub async fn find_all_deleted_hashed_by_sizes(
        transaction: &mut DbTransaction,
        sizes: &[i64],
    ) -> Result<Vec<Book>, DbError> {
        let query = Query::select()
            .columns(COLUMNS)
            .from(B::Table)
            .and_where(Expr::col(B::DeletedDate).is_not_null())
            .and_where(Expr::col(B::FileHash).ne(""))
            .and_where(Expr::col(B::FileSize).is_in(sizes.iter().copied()))
            .take();
        Self::find(transaction, query).await
    }

    pub async fn find_not_deleted_id_by_library_and_url(
        transaction: &mut DbTransaction,
        library_id: &LibraryId,
        url: &FileUrl,
    ) -> Result<Option<BookId>, DbError> {
        let (sql, values) = Query::select()
            .column(B::Id)
            .from(B::Table)
            .and_where(Expr::col(B::LibraryId).eq(library_id))
            .and_where(Expr::col(B::Url).eq(url))
            .and_where(Expr::col(B::DeletedDate).is_null())
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?)
    }

    /// `findFirstIdInSeriesOrNull`: by `NUMBER_SORT`, soft-deleted books included.
    pub async fn find_first_id_in_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Option<BookId>, DbError> {
        let query = in_series_by_number(series_id, Order::Asc);
        find_id(transaction, query).await
    }

    /// `findLastIdInSeriesOrNull`: by `NUMBER_SORT`, soft-deleted books included.
    pub async fn find_last_id_in_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
    ) -> Result<Option<BookId>, DbError> {
        let query = in_series_by_number(series_id, Order::Desc);
        find_id(transaction, query).await
    }

    /// `findFirstUnreadIdInSeriesOrNull`: the first book by `NUMBER_SORT` the user has not
    /// completed, soft-deleted books included.
    pub async fn find_first_unread_id_in_series(
        transaction: &mut DbTransaction,
        series_id: &SeriesId,
        user_id: &UserId,
    ) -> Result<Option<BookId>, DbError> {
        let query = in_series_by_number(series_id, Order::Asc)
            .left_join(
                R::Table,
                Expr::col((B::Table, B::Id))
                    .equals((R::Table, R::BookId))
                    .and(
                        Expr::col((R::Table, R::UserId)).eq(user_id).or(Expr::col((
                            R::Table,
                            R::UserId,
                        ))
                        .is_null()),
                    ),
            )
            .and_where(
                Expr::col((R::Table, R::Completed)).is_null().or(Expr::col((
                    R::Table,
                    R::Completed,
                ))
                .eq(false)),
            )
            .take();
        find_id(transaction, query).await
    }

    pub async fn find_all_not_deleted_ids(
        transaction: &mut DbTransaction,
    ) -> Result<Vec<BookId>, DbError> {
        let (sql, values) = Query::select()
            .column(B::Id)
            .from(B::Table)
            .and_where(Expr::col(B::DeletedDate).is_null())
            .build_sqlx(SqliteQueryBuilder);
        Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?)
    }
}

/// The ID of a series' first book in `order` of `NUMBER_SORT`; ties are left to SQLite, as in
/// Kotlin.
fn in_series_by_number(series_id: &SeriesId, order: Order) -> SelectStatement {
    Query::select()
        .column((B::Table, B::Id))
        .from(B::Table)
        .left_join(
            D::Table,
            Expr::col((B::Table, B::Id)).equals((D::Table, D::BookId)),
        )
        .and_where(Expr::col((B::Table, B::SeriesId)).eq(series_id))
        .order_by((D::Table, D::NumberSort), order)
        .limit(1)
        .take()
}

async fn find_id(
    transaction: &mut DbTransaction,
    query: SelectStatement,
) -> Result<Option<BookId>, DbError> {
    let (sql, values) = query.build_sqlx(SqliteQueryBuilder);
    Ok(sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
        .fetch_optional(transaction.connection())
        .await?)
}

fn with_media(library_id: &LibraryId) -> SelectStatement {
    Query::select()
        .columns(COLUMNS.map(|column| (B::Table, column)))
        .from(B::Table)
        .inner_join(
            M::Table,
            Expr::col((B::Table, B::Id)).equals((M::Table, M::BookId)),
        )
        .and_where(Expr::col((B::Table, B::LibraryId)).eq(library_id))
        .take()
}
