//! Kotlin `ThumbnailReadListDao` and `ThumbnailSeriesCollectionDao`: posters uploaded for read
//! lists and collections, whose tables have the same columns apart from their owner.

use std::fmt::Display;

use crate::iden::{SetKey as K, SetTable as T};
use komga_types::{CollectionId, ContentScope, ReadListId, SetId, ThumbnailDetails, ThumbnailId};
use sea_query::{Expr, ExprTrait, Query, SqliteQueryBuilder};
use sea_query_sqlx::SqlxBinder;
use sqlx::AssertSqlSafe;

use super::{DetailsRow, EntryRow, ThumbnailEntry};
use crate::{DbError, DbTransaction, ReadListDao, SeriesCollectionDao};

/// The ID of a read list or a collection, which posters belong to.
pub trait PosterSet: From<String> + PartialEq + Display + Send + Sync {
    /// The poster table.
    const TABLE: T;
    /// The poster table's column of this ID.
    const OWNER: K;

    fn key(&self) -> &str;

    fn set_id(&self) -> SetId;

    /// Whether the set exists and holds content `scope` sees, or exists at all without a scope.
    fn is_visible(
        &self,
        transaction: &mut DbTransaction,
        scope: Option<&ContentScope>,
    ) -> impl Future<Output = Result<bool, DbError>> + Send;
}

impl PosterSet for ReadListId {
    const TABLE: T = T::ThumbnailReadlist;
    const OWNER: K = K::ReadlistId;

    fn key(&self) -> &str {
        self.as_str()
    }

    fn set_id(&self) -> SetId {
        SetId::ReadList(self.clone())
    }

    async fn is_visible(
        &self,
        transaction: &mut DbTransaction,
        scope: Option<&ContentScope>,
    ) -> Result<bool, DbError> {
        Ok(ReadListDao::find_by_id(transaction, self, scope)
            .await?
            .is_some())
    }
}

impl PosterSet for CollectionId {
    const TABLE: T = T::ThumbnailCollection;
    const OWNER: K = K::CollectionId;

    fn key(&self) -> &str {
        self.as_str()
    }

    fn set_id(&self) -> SetId {
        SetId::Collection(self.clone())
    }

    async fn is_visible(
        &self,
        transaction: &mut DbTransaction,
        scope: Option<&ContentScope>,
    ) -> Result<bool, DbError> {
        Ok(SeriesCollectionDao::find_by_id(transaction, self, scope)
            .await?
            .is_some())
    }
}

pub struct ThumbnailSetDao;

impl ThumbnailSetDao {
    /// `findAllByReadListId` and `findAllByCollectionId`, in table order.
    pub async fn find_all<S: PosterSet>(
        transaction: &mut DbTransaction,
        set: &S,
    ) -> Result<Vec<ThumbnailEntry>, DbError> {
        let (sql, values) = Query::select()
            .columns([K::Id, K::Type, K::Selected])
            .expr_as(Expr::val(true), "STORED")
            .from(S::TABLE)
            .and_where(Expr::col(S::OWNER).eq(set.key()))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<EntryRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(EntryRow::into_entry).collect()
    }

    /// [`Self::find_all`] with the details clients list.
    pub async fn find_all_details<S: PosterSet>(
        transaction: &mut DbTransaction,
        set: &S,
    ) -> Result<Vec<ThumbnailDetails>, DbError> {
        let (sql, values) = Query::select()
            .columns([
                K::Id,
                K::Type,
                K::Selected,
                K::MediaType,
                K::FileSize,
                K::Width,
                K::Height,
            ])
            .from(S::TABLE)
            .and_where(Expr::col(S::OWNER).eq(set.key()))
            .build_sqlx(SqliteQueryBuilder);
        let rows: Vec<DetailsRow> = sqlx::query_as_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_all(transaction.connection())
            .await?;
        rows.into_iter().map(DetailsRow::into_details).collect()
    }

    /// The set a poster belongs to.
    pub async fn find_owner<S: PosterSet>(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<Option<S>, DbError> {
        let (sql, values) = Query::select()
            .column(S::OWNER)
            .from(S::TABLE)
            .and_where(Expr::col(K::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let owner: Option<String> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(owner.map(S::from))
    }

    /// The bytes of `findByIdOrNull`.
    pub async fn find_thumbnail<S: PosterSet>(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<Option<Vec<u8>>, DbError> {
        let (sql, values) = Query::select()
            .column(K::Thumbnail)
            .from(S::TABLE)
            .and_where(Expr::col(K::Id).eq(id))
            .build_sqlx(SqliteQueryBuilder);
        let bytes = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(bytes)
    }

    /// The bytes of `findSelectedByReadListIdOrNull` and `findSelectedByCollectionIdOrNull`: the
    /// first selected row.
    pub async fn find_selected_thumbnail<S: PosterSet>(
        transaction: &mut DbTransaction,
        set: &S,
    ) -> Result<Option<Vec<u8>>, DbError> {
        let (sql, values) = Query::select()
            .column(K::Thumbnail)
            .from(S::TABLE)
            .and_where(Expr::col(S::OWNER).eq(set.key()))
            .and_where(Expr::col(K::Selected).eq(true))
            .limit(1)
            .build_sqlx(SqliteQueryBuilder);
        let bytes = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(bytes)
    }

    /// Inserts `thumbnail` unselected, whatever its `selected`.
    pub async fn insert<S: PosterSet>(
        transaction: &mut DbTransaction,
        set: &S,
        thumbnail: &ThumbnailDetails,
        bytes: &[u8],
    ) -> Result<(), DbError> {
        let (sql, values) = Query::insert()
            .into_table(S::TABLE)
            .columns([
                K::Id,
                S::OWNER,
                K::Thumbnail,
                K::Selected,
                K::Type,
                K::MediaType,
                K::Width,
                K::Height,
                K::FileSize,
            ])
            .values_panic([
                (&thumbnail.id).into(),
                set.key().into(),
                bytes.to_vec().into(),
                false.into(),
                thumbnail.kind.into(),
                thumbnail.media_type.as_str().into(),
                thumbnail.dimension.width.into(),
                thumbnail.dimension.height.into(),
                thumbnail.file_size.into(),
            ])
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// `markSelected`: selects this row and unselects the set's others.
    pub async fn mark_selected<S: PosterSet>(
        transaction: &mut DbTransaction,
        set: &S,
        id: &ThumbnailId,
    ) -> Result<(), DbError> {
        let (sql, values) = Query::update()
            .table(S::TABLE)
            .value(K::Selected, Expr::col(K::Id).eq(id))
            .and_where(Expr::col(S::OWNER).eq(set.key()))
            .build_sqlx(SqliteQueryBuilder);
        sqlx::query_with(AssertSqlSafe(sql.as_str()), values)
            .execute(transaction.connection())
            .await?;
        Ok(())
    }

    /// Returns whether the deleted thumbnail was the selected one.
    pub async fn delete<S: PosterSet>(
        transaction: &mut DbTransaction,
        id: &ThumbnailId,
    ) -> Result<bool, DbError> {
        let (sql, values) = Query::delete()
            .from_table(S::TABLE)
            .and_where(Expr::col(K::Id).eq(id))
            .returning_col(K::Selected)
            .build_sqlx(SqliteQueryBuilder);
        let selected: Option<bool> = sqlx::query_scalar_with(AssertSqlSafe(sql.as_str()), values)
            .fetch_optional(transaction.connection())
            .await?;
        Ok(selected.unwrap_or(false))
    }
}
