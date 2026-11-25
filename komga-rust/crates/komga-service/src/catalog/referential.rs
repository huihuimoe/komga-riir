//! Referential metadata values over visible content (Kotlin `ReferentialRepository`).

use komga_db::{Credit, Database, DbError, ReferentialDao, SeriesValue};
use komga_types::{
    BookAuthor, ContentScope, FilterBy, FilterTags, KomgaUser, NotableEntity, PageRequest,
    PageResult,
};

/// Distinct metadata values, always scoped to what the requesting user may see.
#[derive(Clone)]
pub struct ReferentialService {
    db: Database,
}

impl ReferentialService {
    #[must_use]
    pub fn new(db: Database) -> Self {
        Self { db }
    }

    /// `findGenres`, `findSharingLabels`, `findLanguages` and `findPublishers`.
    pub async fn series_values(
        &self,
        user: &KomgaUser,
        kind: SeriesValue,
        search: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_series_values(&self.db, &scope, kind, search, filter, page).await
    }

    /// `findAgeRatings`.
    pub async fn age_ratings(
        &self,
        user: &KomgaUser,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<i32>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_age_ratings(&self.db, &scope, filter, page).await
    }

    /// Deprecated `findAllAgeRatings*`: `None` when a series is unrated.
    pub async fn age_ratings_with_unrated(
        &self,
        user: &KomgaUser,
        filter: Option<&FilterBy>,
    ) -> Result<Vec<Option<i32>>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_age_ratings_with_unrated(&self.db, &scope, filter).await
    }

    /// `findSeriesReleaseYears`.
    pub async fn series_release_years(
        &self,
        user: &KomgaUser,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_series_release_years(&self.db, &scope, filter, page).await
    }

    /// `findAuthors`.
    pub async fn authors(
        &self,
        user: &KomgaUser,
        search: Option<&str>,
        role: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<BookAuthor>, DbError> {
        let scope = ContentScope::for_user(user);
        let credit = Credit::Author;
        let authors =
            ReferentialDao::find_credits(&self.db, &scope, credit, search, role, filter, page)
                .await?;
        Ok(PageResult {
            items: authors
                .items
                .into_iter()
                .map(|(name, role)| BookAuthor { name, role })
                .collect(),
            total_elements: authors.total_elements,
        })
    }

    /// `findNotableEntities`.
    pub async fn notable_entities(
        &self,
        user: &KomgaUser,
        search: Option<&str>,
        kind: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<NotableEntity>, DbError> {
        let scope = ContentScope::for_user(user);
        let credit = Credit::NotableEntity;
        let entities =
            ReferentialDao::find_credits(&self.db, &scope, credit, search, kind, filter, page)
                .await?;
        Ok(PageResult {
            items: entities
                .items
                .into_iter()
                .map(|(name, kind)| NotableEntity { kind, name })
                .collect(),
            total_elements: entities.total_elements,
        })
    }

    /// `findAuthorsNames` and `findNotableEntitiesNames`.
    pub async fn credit_names(
        &self,
        user: &KomgaUser,
        credit: Credit,
        search: Option<&str>,
        kind: Option<&str>,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_credit_names(&self.db, &scope, credit, search, kind, filter, page)
            .await
    }

    /// `findAuthorsRoles` and `findNotableEntitiesKinds`.
    pub async fn credit_kinds(
        &self,
        user: &KomgaUser,
        credit: Credit,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_credit_kinds(&self.db, &scope, credit, filter, page).await
    }

    /// `findTags`.
    pub async fn tags(
        &self,
        user: &KomgaUser,
        search: Option<&str>,
        source: FilterTags,
        filter: Option<&FilterBy>,
        page: PageRequest,
    ) -> Result<PageResult<String>, DbError> {
        let scope = ContentScope::for_user(user);
        ReferentialDao::find_tags(&self.db, &scope, search, source, filter, page).await
    }
}
