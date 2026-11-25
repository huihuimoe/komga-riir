//! Kotlin `DomainEvent`.

use crate::domain::{Book, ReadList, Series, SeriesCollection};
use crate::ids::{BookId, CollectionId, LibraryId, ReadListId, SeriesId, UserId};

/// A change to the content, published once the transaction making it has committed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DomainEvent {
    LibraryAdded(LibraryId),
    LibraryUpdated(LibraryId),
    LibraryDeleted(LibraryId),
    SeriesAdded(SeriesRef),
    SeriesUpdated(SeriesRef),
    SeriesDeleted(SeriesRef),
    BookAdded(BookRef),
    BookUpdated(BookRef),
    BookDeleted(BookRef),
    ReadListAdded(ReadListRef),
    ReadListUpdated(ReadListRef),
    ReadListDeleted(ReadListRef),
    CollectionAdded(CollectionRef),
    CollectionUpdated(CollectionRef),
    CollectionDeleted(CollectionRef),
    ReadProgressChanged(BookProgressRef),
    ReadProgressDeleted(BookProgressRef),
    ReadProgressSeriesChanged(SeriesProgressRef),
    ReadProgressSeriesDeleted(SeriesProgressRef),
    ThumbnailBookAdded(ThumbnailRef<BookRef>),
    ThumbnailBookDeleted(ThumbnailRef<BookRef>),
    ThumbnailSeriesAdded(ThumbnailRef<SeriesRef>),
    ThumbnailSeriesDeleted(ThumbnailRef<SeriesRef>),
    ThumbnailSetAdded(ThumbnailRef<SetId>),
    ThumbnailSetDeleted(ThumbnailRef<SetId>),
    /// The user's sessions were revoked, as the user changed or was deleted.
    SessionsExpired(UserId),
    /// An import task ended; admins only.
    BookImported(BookImport),
}

/// The outcome of importing `source_file`: the book, or the code or message of the failure.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookImport {
    pub book_id: Option<BookId>,
    pub source_file: String,
    pub message: Option<String>,
}

/// A thumbnail of `owner`, and whether it is the owner's selected one.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbnailRef<T> {
    pub owner: T,
    pub selected: bool,
}

/// A read list or a collection.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum SetId {
    ReadList(ReadListId),
    Collection(CollectionId),
}

/// A series and the library it belongs to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesRef {
    pub series_id: SeriesId,
    pub library_id: LibraryId,
}

/// A book and where it belongs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookRef {
    pub book_id: BookId,
    pub series_id: SeriesId,
    pub library_id: LibraryId,
}

/// A user's progress on a book.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BookProgressRef {
    pub book_id: BookId,
    pub user_id: UserId,
}

/// A user's progress on a series, aggregated from their books'.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SeriesProgressRef {
    pub series_id: SeriesId,
    pub user_id: UserId,
}

/// A read list and its books.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ReadListRef {
    pub read_list_id: ReadListId,
    pub book_ids: Vec<BookId>,
}

/// A collection and its series.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CollectionRef {
    pub collection_id: CollectionId,
    pub series_ids: Vec<SeriesId>,
}

impl From<&ReadList> for ReadListRef {
    fn from(read_list: &ReadList) -> Self {
        Self {
            read_list_id: read_list.id.clone(),
            book_ids: read_list.book_ids.clone(),
        }
    }
}

impl From<&SeriesCollection> for CollectionRef {
    fn from(collection: &SeriesCollection) -> Self {
        Self {
            collection_id: collection.id.clone(),
            series_ids: collection.series_ids.clone(),
        }
    }
}

impl From<&Series> for SeriesRef {
    fn from(series: &Series) -> Self {
        Self {
            series_id: series.id.clone(),
            library_id: series.library_id.clone(),
        }
    }
}

impl From<&Book> for BookRef {
    fn from(book: &Book) -> Self {
        Self {
            book_id: book.id.clone(),
            series_id: book.series_id.clone(),
            library_id: book.library_id.clone(),
        }
    }
}
