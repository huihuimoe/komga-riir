//! Kotlin `ReadListLifecycle`. The search index follows every change of a name or of which lists
//! exist.

use komga_db::{
    BookDao, Database, DbError, DbTransaction, ReadListContent, ReadListDao, ReadListRequestDao,
    SearchIndexPendingDao,
};
use komga_types::{
    Book, BookId, ContentScope, DomainEvent, KomgaUser, LibraryId, PageRequest, PageResult,
    ReadList, ReadListId, ReadListRef, ReadListRequestBook, ReadListSeriesMatch, ReadListSort,
    SearchEntity, SortOrder,
};
use thiserror::Error;
use tracing::{debug, info};

use crate::catalog::search_index::{FullTextSearch, SearchIndexError, SearchIndexNotifier};
use crate::events::EventBus;
use crate::support::access_policy::{self, Access};
use cbl::ComicRackListError;

mod archive;
mod cbl;

#[derive(Debug, Error)]
pub enum ReadListError {
    #[error("Read list not found")]
    NotFound,
    #[error("Forbidden")]
    Forbidden,
    #[error("Book not found")]
    BookNotFound,
    /// `ContentRestrictionChecker` denied the book.
    #[error("Content restricted")]
    Restricted,
    /// `DuplicateNameException`.
    #[error("Read list name already exists")]
    DuplicateName,
    #[error("Some books do not exist")]
    UnknownBook,
    #[error("Invalid ComicRack list: {0}")]
    ComicRackList(#[from] ComicRackListError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
    #[error(transparent)]
    Search(#[from] SearchIndexError),
}

/// A read list to create.
#[derive(Debug, Clone)]
pub struct NewReadList {
    pub name: String,
    pub summary: String,
    pub ordered: bool,
    /// In reading order, without duplicates.
    pub book_ids: Vec<BookId>,
}

/// Changes to a read list; `None` keeps the current value.
#[derive(Debug, Clone)]
pub struct ReadListUpdate {
    pub name: Option<String>,
    pub summary: Option<String>,
    pub ordered: Option<bool>,
    /// Replaces every book, in reading order, without duplicates.
    pub book_ids: Option<Vec<BookId>>,
}

/// Kotlin `ReadListRequestMatch`.
#[derive(Debug, Clone)]
pub struct ReadListRequestMatch {
    pub name: String,
    /// A read list already has this name, ignoring ASCII case.
    pub duplicate_name: bool,
    /// One per requested book, in request order.
    pub requests: Vec<ReadListMatch>,
}

/// Kotlin `ReadListRequestBookMatches`.
#[derive(Debug, Clone)]
pub struct ReadListMatch {
    pub request: ReadListRequestBook,
    pub matches: Vec<ReadListSeriesMatch>,
}

#[derive(Clone)]
pub struct ReadListService {
    db: Database,
    search: SearchIndexNotifier,
    full_text: FullTextSearch,
    events: EventBus,
}

impl ReadListService {
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

    /// The read list as `user` sees it.
    pub async fn get(&self, user: &KomgaUser, id: &ReadListId) -> Result<ReadList, ReadListError> {
        let mut tx = self.db.begin().await?;
        let scope = ContentScope::for_user(user);
        let found = ReadListDao::find_by_id(&mut tx, id, Some(&scope)).await?;
        tx.commit().await?;
        found.ok_or(ReadListError::NotFound)
    }

    /// `getReadListsByBookId`: the read lists holding a book `user` may see, as they see them.
    pub async fn containing_book(
        &self,
        user: &KomgaUser,
        book_id: &BookId,
    ) -> Result<Vec<ReadList>, ReadListError> {
        let mut tx = self.db.begin().await?;
        match access_policy::check_book_id(&mut tx, user, book_id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(ReadListError::Restricted),
            Access::Unknown => return Err(ReadListError::BookNotFound),
        }
        let scope = ContentScope::for_user(user);
        let found = ReadListDao::find_all_containing_book(&mut tx, book_id, &scope).await?;
        tx.commit().await?;
        Ok(found)
    }

    /// The read lists `user` sees, holding a book of `libraries` and matching the full-text
    /// search `full_text`, which is not blank, when given.
    pub async fn list(
        &self,
        user: &KomgaUser,
        libraries: Option<&[LibraryId]>,
        full_text: Option<&str>,
        sort: &[SortOrder<ReadListSort>],
        page: PageRequest,
    ) -> Result<PageResult<ReadList>, ReadListError> {
        let mut search = self
            .full_text
            .begin(&self.db, SearchEntity::ReadList, full_text)
            .await?;
        let scope = ContentScope::for_user(user);
        let found = ReadListDao::find_all(
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

    /// `addReadList`: the result holds every book, whoever creates it.
    pub async fn create(&self, new: NewReadList) -> Result<ReadList, ReadListError> {
        info!(name = %new.name, books = new.book_ids.len(), "Adding new read list");
        let mut tx = self.db.begin_immediate().await?;
        if ReadListDao::exists_by_name(&mut tx, &new.name, None).await? {
            return Err(ReadListError::DuplicateName);
        }
        check_books_exist(&mut tx, &new.book_ids).await?;
        let id = ReadListId::generate();
        let content = ReadListContent {
            name: &new.name,
            summary: &new.summary,
            ordered: new.ordered,
        };
        ReadListDao::insert(&mut tx, &id, &content, &new.book_ids).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &[id.as_str()]).await?;
        let created = ReadListDao::find_by_id(&mut tx, &id, None)
            .await?
            .expect("The read list was inserted in this transaction");
        tx.record(DomainEvent::ReadListAdded(ReadListRef::from(&created)));
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(created)
    }

    /// `updateReadList` of a read list `user` sees. Unlike Kotlin, which rewrites the books a
    /// restricted user sees when none are given, the books change only when given.
    pub async fn update(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
        update: ReadListUpdate,
    ) -> Result<(), ReadListError> {
        info!(read_list = %id, ?update, "Update read list");
        let mut tx = self.db.begin_immediate().await?;
        let scope = ContentScope::for_user(user);
        let existing = ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(ReadListError::NotFound)?;
        if let Some(name) = &update.name
            && ReadListDao::exists_by_name(&mut tx, name, Some(id)).await?
        {
            return Err(ReadListError::DuplicateName);
        }
        if let Some(book_ids) = &update.book_ids {
            check_books_exist(&mut tx, book_ids).await?;
        }
        let content = ReadListContent {
            name: update.name.as_deref().unwrap_or(&existing.name),
            summary: update.summary.as_deref().unwrap_or(&existing.summary),
            ordered: update.ordered.unwrap_or(existing.ordered),
        };
        ReadListDao::update(&mut tx, id, &content, update.book_ids.as_deref()).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &[id.as_str()]).await?;
        record_updated(&mut tx, id).await?;
        self.events.commit(tx).await?;
        self.search.notify();
        Ok(())
    }

    /// `deleteReadList` of a read list `user` sees, with its thumbnails.
    pub async fn delete(&self, user: &KomgaUser, id: &ReadListId) -> Result<(), ReadListError> {
        let mut tx = self.db.begin_immediate().await?;
        let scope = ContentScope::for_user(user);
        if ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .is_none()
        {
            return Err(ReadListError::NotFound);
        }
        let deleted = ReadListDao::find_by_id(&mut tx, id, None)
            .await?
            .expect("The read list was found in this transaction");
        ReadListDao::delete(&mut tx, id).await?;
        SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &[id.as_str()]).await?;
        tx.record(DomainEvent::ReadListDeleted(ReadListRef::from(&deleted)));
        self.events.commit(tx).await?;
        self.search.notify();
        info!(read_list = %id, "Deleted read list");
        Ok(())
    }

    /// `matchComicRackList`: the books of every library that may be those of the ComicRack
    /// reading list `content`.
    pub async fn match_comic_rack_list(
        &self,
        content: &[u8],
    ) -> Result<ReadListRequestMatch, ReadListError> {
        let request = cbl::import_from_cbl(content)?;
        info!(name = %request.name, books = request.books.len(), "Trying to match read list request");
        let mut tx = self.db.begin().await?;
        let duplicate_name = ReadListDao::exists_by_name(&mut tx, &request.name, None).await?;
        let matches = ReadListRequestDao::match_book_requests(&mut tx, &request.books).await?;
        tx.commit().await?;
        let requests = request
            .books
            .into_iter()
            .zip(matches)
            .map(|(request, matches)| ReadListMatch { request, matches })
            .collect();
        Ok(ReadListRequestMatch {
            name: request.name,
            duplicate_name,
            requests,
        })
    }
}

/// Deletes every read list without books.
pub(crate) async fn delete_empty(
    db: &Database,
    search: &SearchIndexNotifier,
    events: &EventBus,
) -> Result<(), DbError> {
    let mut tx = db.begin_immediate().await?;
    let deleted = ReadListDao::delete_empty(&mut tx).await?;
    SearchIndexPendingDao::mark(&mut tx, SearchEntity::ReadList, &deleted).await?;
    for id in &deleted {
        tx.record(DomainEvent::ReadListDeleted(ReadListRef {
            read_list_id: id.clone(),
            book_ids: Vec::new(),
        }));
    }
    events.commit(tx).await?;
    search.notify();
    info!(read_lists = ?deleted, "Deleted empty read lists");
    Ok(())
}

/// `addBookToReadList`: adds `book` to the read list called `name`, creating the list when none
/// has that name. Without a `number`, or when another book holds it, the book goes after the last
/// one. A new list is marked for the search index.
pub(crate) async fn add_book(
    transaction: &mut DbTransaction,
    name: &str,
    book: &Book,
    number: Option<i32>,
) -> Result<(), DbError> {
    let Some(existing) = ReadListDao::find_by_name(transaction, name).await? else {
        debug!(book = %book.name, read_list = name, "Adding book to new read list");
        let id = ReadListId::generate();
        ReadListDao::insert_with_book(transaction, &id, name, number.unwrap_or(0), &book.id)
            .await?;
        transaction.record(DomainEvent::ReadListAdded(ReadListRef {
            read_list_id: id.clone(),
            book_ids: vec![book.id.clone()],
        }));
        return SearchIndexPendingDao::mark(transaction, SearchEntity::ReadList, &[id.as_str()])
            .await;
    };
    if existing.books.iter().any(|(_, id)| *id == book.id) {
        debug!(read_list = name, "Book is already in existing read list");
        return Ok(());
    }
    // Kotlin fails on a list emptied since it was found; such a list starts again at 0.
    let next = existing
        .books
        .last()
        .map_or(0, |(last, _)| last.saturating_add(1));
    let number = match number {
        Some(number) if !existing.books.iter().any(|(taken, _)| *taken == number) => number,
        _ => next,
    };
    debug!(book = %book.name, read_list = name, number, "Adding book to existing read list");
    ReadListDao::add_book(transaction, &existing.id, number, &book.id).await?;
    record_updated(transaction, &existing.id).await
}

/// Records the update of read list `id`, with its books as they now are.
async fn record_updated(transaction: &mut DbTransaction, id: &ReadListId) -> Result<(), DbError> {
    let updated = ReadListDao::find_by_id(transaction, id, None)
        .await?
        .expect("The read list was found in this transaction");
    transaction.record(DomainEvent::ReadListUpdated(ReadListRef::from(&updated)));
    Ok(())
}

/// Kotlin fails on the foreign key of an unknown book with a server error; it is a client error
/// here.
async fn check_books_exist(
    transaction: &mut DbTransaction,
    book_ids: &[BookId],
) -> Result<(), ReadListError> {
    if BookDao::count_by_ids(transaction, book_ids).await? == book_ids.len() {
        Ok(())
    } else {
        Err(ReadListError::UnknownBook)
    }
}
