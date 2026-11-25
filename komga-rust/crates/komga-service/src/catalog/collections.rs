//! Kotlin `SeriesCollectionLifecycle`.

use komga_db::{
    Database, DbError, DbTransaction, SearchIndexPendingDao, SeriesCollectionDao, SeriesDao,
};
use komga_types::{
    CollectionId, CollectionRef, CollectionSort, ContentScope, DomainEvent, KomgaUser, LibraryId,
    PageRequest, PageResult, SearchEntity, Series, SeriesCollection, SeriesId, SortOrder,
};
use thiserror::Error;
use tracing::{debug, info};

use crate::catalog::search_index::{FullTextSearch, SearchIndexError, SearchIndexNotifier};
use crate::events::EventBus;
use crate::support::access_policy::{self, Access};

#[derive(Debug, Error)]
pub enum CollectionError {
    #[error("Collection not found")]
    NotFound,
    #[error("Series not found")]
    SeriesNotFound,
    /// `ContentRestrictionChecker` denied the series.
    #[error("Content restricted")]
    Restricted,
    /// `DuplicateNameException`.
    #[error("Collection name already exists")]
    DuplicateName,
    #[error("Some series do not exist")]
    UnknownSeries,
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Search(#[from] SearchIndexError),
}

/// A collection to create.
#[derive(Debug, Clone)]
pub struct NewCollection {
    pub name: String,
    pub ordered: bool,
    /// In collection order, without duplicates.
    pub series_ids: Vec<SeriesId>,
}

/// Changes to a collection; `None` keeps the current value.
#[derive(Debug, Clone)]
pub struct CollectionUpdate {
    pub name: Option<String>,
    pub ordered: Option<bool>,
    /// Replaces every series, in collection order, without duplicates.
    pub series_ids: Option<Vec<SeriesId>>,
}

#[derive(Clone)]
pub struct SeriesCollectionService {
    db: Database,
    search: SearchIndexNotifier,
    full_text: FullTextSearch,
    events: EventBus,
}

impl SeriesCollectionService {
    #[must_use]
    pub fn new(
        db: Database,
        search: SearchIndexNotifier,
        full_text: FullTextSearch,
        events: EventBus,
    ) -> Self {
        Self {
            db,
            search,
            full_text,
            events,
        }
    }

    /// The collection as `user` sees it.
    pub async fn get(
        &self,
        user: &KomgaUser,
        id: &CollectionId,
    ) -> Result<SeriesCollection, CollectionError> {
        let mut tx = self.db.begin().await?;
        let scope = ContentScope::for_user(user);
        let found = SeriesCollectionDao::find_by_id(&mut tx, id, Some(&scope)).await?;
        tx.commit().await?;
        found.ok_or(CollectionError::NotFound)
    }

    /// `getCollectionsBySeriesId`: the collections holding a series `user` may see, as they see
    /// them.
    pub async fn containing_series(
        &self,
        user: &KomgaUser,
        series_id: &SeriesId,
    ) -> Result<Vec<SeriesCollection>, CollectionError> {
        let mut tx = self.db.begin().await?;
        match access_policy::check_series_id(&mut tx, user, series_id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(CollectionError::Restricted),
            Access::Unknown => return Err(CollectionError::SeriesNotFound),
        }
        let scope = ContentScope::for_user(user);
        let found =
            SeriesCollectionDao::find_all_containing_series(&mut tx, series_id, &scope).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// The collections `user` sees, holding a series of `libraries` and matching the full-text
    /// search `full_text`, which is not blank, when given.
    pub async fn list(
        &self,
        user: &KomgaUser,
        libraries: Option<&[LibraryId]>,
        full_text: Option<&str>,
        sort: &[SortOrder<CollectionSort>],
        page: PageRequest,
    ) -> Result<PageResult<SeriesCollection>, CollectionError> {
        let mut search = self
            .full_text
            .begin(&self.db, SearchEntity::Collection, full_text)
            .await?;
        let scope = ContentScope::for_user(user);
        let found = SeriesCollectionDao::find_all(
            &mut search.transaction,
            &scope,
            libraries,
            search.candidates.as_ref(),
            sort,
            page,
        )
        .await?;
        search.transaction.rollback().await?;
        Ok(found)
    }

    /// `addCollection`: the result holds every series, whoever creates it.
    pub async fn create(&self, new: NewCollection) -> Result<SeriesCollection, CollectionError> {
        info!(name = %new.name, series = new.series_ids.len(), "Adding new collection");
        let mut tx = self.db.begin_immediate().await?;
        if SeriesCollectionDao::exists_by_name(&mut tx, &new.name, None).await? {
            return Err(CollectionError::DuplicateName);
        }
        check_series_exist(&mut tx, &new.series_ids).await?;
        let id = CollectionId::generate();
        SeriesCollectionDao::insert(&mut tx, &id, &new.name, new.ordered, &new.series_ids).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Collection, &[id.as_str()]).await?;
        let created = SeriesCollectionDao::find_by_id(&mut tx, &id, None)
            .await?
            .expect("The collection was inserted in this transaction");
        tx.record(DomainEvent::CollectionAdded(CollectionRef::from(&created)));
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(created)
    }

    /// `updateCollection` of a collection `user` sees. Unlike Kotlin, which rewrites the series a
    /// restricted user sees when none are given, the series change only when given.
    pub async fn update(
        &self,
        user: &KomgaUser,
        id: &CollectionId,
        update: CollectionUpdate,
    ) -> Result<(), CollectionError> {
        info!(collection = %id, ?update, "Update collection");
        let mut tx = self.db.begin_immediate().await?;
        let scope = ContentScope::for_user(user);
        let existing = SeriesCollectionDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(CollectionError::NotFound)?;
        if let Some(name) = &update.name
            && SeriesCollectionDao::exists_by_name(&mut tx, name, Some(id)).await?
        {
            return Err(CollectionError::DuplicateName);
        }
        if let Some(series_ids) = &update.series_ids {
            check_series_exist(&mut tx, series_ids).await?;
        }
        let name = update.name.as_deref().unwrap_or(&existing.name);
        let ordered = update.ordered.unwrap_or(existing.ordered);
        let series_ids = update.series_ids.as_deref();
        SeriesCollectionDao::update(&mut tx, id, name, ordered, series_ids).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Collection, &[id.as_str()]).await?;
        record_updated(&mut tx, id).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }

    /// `deleteCollection` of a collection `user` sees, with its thumbnails.
    pub async fn delete(&self, user: &KomgaUser, id: &CollectionId) -> Result<(), CollectionError> {
        let mut tx = self.db.begin_immediate().await?;
        let scope = ContentScope::for_user(user);
        if SeriesCollectionDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .is_none()
        {
            return Err(CollectionError::NotFound);
        }
        let deleted = SeriesCollectionDao::find_by_id(&mut tx, id, None)
            .await?
            .expect("The collection was found in this transaction");
        SeriesCollectionDao::delete(&mut tx, id).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::Collection, &[id.as_str()]).await?;
        tx.record(DomainEvent::CollectionDeleted(CollectionRef::from(
            &deleted,
        )));
        self.events.commit(tx).await?;
        self.search.notify();
        info!(collection = %id, "Deleted collection");
        Ok(())
    }
}

/// Deletes every collection without series.
pub(crate) async fn delete_empty(
    db: &Database,
    search: &SearchIndexNotifier,
    events: &EventBus,
) -> Result<(), DbError> {
    let mut tx = db.begin_immediate().await?;
    let deleted = SeriesCollectionDao::delete_empty(&mut tx).await?;
    SearchIndexPendingDao::mark(&mut tx, SearchEntity::Collection, &deleted).await?;
    for id in &deleted {
        tx.record(DomainEvent::CollectionDeleted(CollectionRef {
            collection_id: id.clone(),
            series_ids: Vec::new(),
        }));
    }
    events.commit(tx).await?;
    search.notify();
    info!(collections = ?deleted, "Deleted empty collections");
    Ok(())
}

/// `addSeriesToCollection`: adds `series` after the last series of the collection called `name`,
/// creating the collection when none has that name; a new one is marked for the search index.
pub(crate) async fn add_series(
    transaction: &mut DbTransaction,
    name: &str,
    series: &Series,
) -> Result<(), DbError> {
    let Some(existing) = SeriesCollectionDao::find_by_name(transaction, name).await? else {
        debug!(series = %series.name, collection = name, "Adding series to new collection");
        let id = CollectionId::generate();
        SeriesCollectionDao::insert_with_series(transaction, &id, name, &series.id).await?;
        transaction.record(DomainEvent::CollectionAdded(CollectionRef {
            collection_id: id.clone(),
            series_ids: vec![series.id.clone()],
        }));
        return SearchIndexPendingDao::mark(transaction, SearchEntity::Collection, &[id.as_str()])
            .await;
    };
    if existing.series.iter().any(|(_, id)| *id == series.id) {
        debug!(
            collection = name,
            "Series is already in existing collection"
        );
        return Ok(());
    }
    let number = existing
        .series
        .last()
        .map_or(0, |(last, _)| last.saturating_add(1));
    debug!(series = %series.name, collection = name, "Adding series to existing collection");
    SeriesCollectionDao::add_series(transaction, &existing.id, number, &series.id).await?;
    record_updated(transaction, &existing.id).await
}

/// Records the update of collection `id`, with its series as they now are.
async fn record_updated(transaction: &mut DbTransaction, id: &CollectionId) -> Result<(), DbError> {
    let updated = SeriesCollectionDao::find_by_id(transaction, id, None)
        .await?
        .expect("The collection was found in this transaction");
    transaction.record(DomainEvent::CollectionUpdated(CollectionRef::from(
        &updated,
    )));
    Ok(())
}

/// Kotlin fails on the foreign key of an unknown series with a server error; it is a client
/// error here.
async fn check_series_exist(
    transaction: &mut DbTransaction,
    series_ids: &[SeriesId],
) -> Result<(), CollectionError> {
    if SeriesDao::count_by_ids(transaction, series_ids).await? == series_ids.len() {
        Ok(())
    } else {
        Err(CollectionError::UnknownSeries)
    }
}
