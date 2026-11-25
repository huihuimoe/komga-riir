use komga_db::SeriesDtoDao;
use komga_types::{
    ContentScope, GroupCount, KomgaUser, PageRequest, PageResult, SearchEntity, SeriesCondition,
    SeriesDetails, SeriesId, SeriesSort, SortOrder,
};

use super::{CatalogError, CatalogService};

impl CatalogService {
    /// `getSeriesById`: the series as `user` sees it, checked against its own metadata like
    /// `checkContentRestrictionSeries(user, SeriesDto)`.
    pub async fn series(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<SeriesDetails, CatalogError> {
        let mut tx = self.db.begin().await?;
        let found = SeriesDtoDao::find_by_id(&mut tx, id, &user.id).await?;
        tx.rollback().await?;
        let series = found.ok_or(CatalogError::NotFound)?;
        let metadata = &series.metadata;
        if !user.can_access_library(&series.series.library_id)
            || !user.is_content_allowed(metadata.age_rating, &metadata.sharing_labels)
        {
            return Err(CatalogError::Restricted);
        }
        Ok(series)
    }

    /// `findAll(SeriesSearch, SearchContext(user), pageable)`, with the full-text search
    /// `full_text` when given, which is not blank.
    pub async fn series_list(
        &self,
        user: &KomgaUser,
        condition: Option<&SeriesCondition>,
        full_text: Option<&str>,
        sort: &[SortOrder<SeriesSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut search = self
            .full_text
            .begin(&self.db, SearchEntity::Series, full_text)
            .await?;
        let series = SeriesDtoDao::find_all(
            &mut search.transaction,
            &user.id,
            &scope,
            condition,
            search.candidates.as_ref(),
            sort,
            page,
        )
        .await?;
        search.transaction.rollback().await?;
        Ok(series)
    }

    /// `findAllRecentlyUpdated`: like [`series_list`](Self::series_list), without the series
    /// never modified since their creation.
    pub async fn updated_series(
        &self,
        user: &KomgaUser,
        condition: Option<&SeriesCondition>,
        sort: &[SortOrder<SeriesSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let series =
            SeriesDtoDao::find_recently_updated(&mut tx, &user.id, &scope, condition, sort, page)
                .await?;
        tx.rollback().await?;
        Ok(series)
    }

    /// `countByFirstCharacter(SeriesSearch, SearchContext(user))`, with the full-text search
    /// `full_text` when given, which is not blank.
    pub async fn series_first_characters(
        &self,
        user: &KomgaUser,
        condition: Option<&SeriesCondition>,
        full_text: Option<&str>,
    ) -> Result<Vec<GroupCount>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut search = self
            .full_text
            .begin(&self.db, SearchEntity::Series, full_text)
            .await?;
        let groups = SeriesDtoDao::count_by_first_character(
            &mut search.transaction,
            &user.id,
            &scope,
            condition,
            search.candidates.as_ref(),
        )
        .await?;
        search.transaction.rollback().await?;
        Ok(groups)
    }
}
