//! The catalog queries both OPDS versions answer, so that they list the same books and series.

use komga_types::{
    BookCondition, BookDetails, BookSort, BooleanOperator, CollectionId, CollectionSort, Equality,
    KomgaUser, LibraryId, MediaStatus, PageRequest, PageResult, ReadList, ReadListId, ReadListSort,
    ReadStatus, SeriesCollection, SeriesCondition, SeriesDetails, SeriesId, SeriesSort,
    SortDirection, SortOrder, StringOperator,
};

use super::OpdsState;
use crate::error::ApiError;

fn order<K>(key: K, direction: SortDirection) -> [SortOrder<K>; 1] {
    [SortOrder { key, direction }]
}

/// Books that can be read: analyzed and still on disk.
fn ready_books(mut conditions: Vec<BookCondition>) -> BookCondition {
    conditions.push(BookCondition::MediaStatus(Equality::Is {
        value: MediaStatus::Ready,
    }));
    conditions.push(BookCondition::Deleted(BooleanOperator::IsFalse));
    BookCondition::AllOf(conditions)
}

fn book_library(library: Option<&LibraryId>) -> Vec<BookCondition> {
    library
        .map(|id| {
            BookCondition::LibraryId(Equality::Is {
                value: id.to_string(),
            })
        })
        .into_iter()
        .collect()
}

/// Series of any of `publishers`; `None` when there are none.
pub(super) fn any_publisher(publishers: &[String]) -> Option<SeriesCondition> {
    (!publishers.is_empty()).then(|| {
        SeriesCondition::AnyOf(
            publishers
                .iter()
                .map(|publisher| {
                    SeriesCondition::Publisher(Equality::Is {
                        value: publisher.clone(),
                    })
                })
                .collect(),
        )
    })
}

pub(super) fn series_library(id: &LibraryId) -> SeriesCondition {
    SeriesCondition::LibraryId(Equality::Is {
        value: id.to_string(),
    })
}

/// The queries; errors are answered for `path`.
pub(super) struct Query<'a> {
    pub(super) state: &'a OpdsState,
    pub(super) user: &'a KomgaUser,
    pub(super) page: PageRequest,
    pub(super) path: &'a str,
}

impl Query<'_> {
    fn catalog_error(&self) -> impl Fn(komga_service::CatalogError) -> ApiError + '_ {
        |error| ApiError::from_catalog(&error, self.path)
    }

    async fn books(
        &self,
        condition: BookCondition,
        sort: [SortOrder<BookSort>; 1],
    ) -> Result<PageResult<BookDetails>, ApiError> {
        self.state
            .catalog
            .books(self.user, Some(&condition), None, &sort, self.page)
            .await
            .map_err(self.catalog_error())
    }

    pub(super) async fn keep_reading(
        &self,
        library: Option<&LibraryId>,
    ) -> Result<PageResult<BookDetails>, ApiError> {
        let mut conditions = book_library(library);
        conditions.push(BookCondition::ReadStatus(Equality::Is {
            value: ReadStatus::InProgress,
        }));
        let sort = order(BookSort::ReadDate, SortDirection::Desc);
        self.books(ready_books(conditions), sort).await
    }

    pub(super) async fn on_deck(
        &self,
        library: Option<&LibraryId>,
    ) -> Result<PageResult<BookDetails>, ApiError> {
        self.state
            .catalog
            .books_on_deck(self.user, library.map(std::slice::from_ref), self.page)
            .await
            .map_err(self.catalog_error())
    }

    pub(super) async fn latest_books(
        &self,
        library: Option<&LibraryId>,
    ) -> Result<PageResult<BookDetails>, ApiError> {
        let sort = order(BookSort::CreatedDate, SortDirection::Desc);
        self.books(ready_books(book_library(library)), sort).await
    }

    /// Books whose title contains every term, analyzed or not, by title.
    pub(super) async fn books_titled(
        &self,
        terms: &[&str],
    ) -> Result<PageResult<BookDetails>, ApiError> {
        let mut conditions: Vec<_> = terms
            .iter()
            .map(|term| {
                BookCondition::Title(StringOperator::Contains {
                    value: (*term).to_owned(),
                })
            })
            .collect();
        conditions.push(BookCondition::Deleted(BooleanOperator::IsFalse));
        let sort = order(BookSort::Title, SortDirection::Asc);
        self.books(BookCondition::AllOf(conditions), sort).await
    }

    async fn series(
        &self,
        mut conditions: Vec<SeriesCondition>,
        sort: [SortOrder<SeriesSort>; 1],
    ) -> Result<PageResult<SeriesDetails>, ApiError> {
        conditions.push(SeriesCondition::Deleted(BooleanOperator::IsFalse));
        let condition = SeriesCondition::AllOf(conditions);
        self.state
            .catalog
            .series_list(self.user, Some(&condition), None, &sort, self.page)
            .await
            .map_err(self.catalog_error())
    }

    /// Series matching `conditions`, by title.
    pub(super) async fn series_by_title(
        &self,
        conditions: Vec<SeriesCondition>,
    ) -> Result<PageResult<SeriesDetails>, ApiError> {
        self.series(conditions, order(SeriesSort::TitleSort, SortDirection::Asc))
            .await
    }

    pub(super) async fn latest_series(
        &self,
        conditions: Vec<SeriesCondition>,
    ) -> Result<PageResult<SeriesDetails>, ApiError> {
        let sort = order(SeriesSort::LastModifiedDate, SortDirection::Desc);
        self.series(conditions, sort).await
    }

    pub(super) async fn series_books(
        &self,
        id: &SeriesId,
        mut conditions: Vec<BookCondition>,
    ) -> Result<PageResult<BookDetails>, ApiError> {
        conditions = vec![ready_books(conditions)];
        let sort = order(BookSort::NumberSort, SortDirection::Asc);
        self.state
            .catalog
            .series_books(self.user, id, conditions, &sort, self.page)
            .await
            .map_err(self.catalog_error())
    }

    pub(super) async fn collection_series(
        &self,
        id: &CollectionId,
    ) -> Result<PageResult<SeriesDetails>, ApiError> {
        let filters = vec![SeriesCondition::Deleted(BooleanOperator::IsFalse)];
        self.state
            .catalog
            .collection_series(self.user, id, filters, self.page)
            .await
            .map_err(self.catalog_error())
    }

    pub(super) async fn read_list_books(
        &self,
        id: &ReadListId,
    ) -> Result<PageResult<BookDetails>, ApiError> {
        self.state
            .catalog
            .read_list_books(self.user, id, vec![ready_books(Vec::new())], self.page)
            .await
            .map_err(self.catalog_error())
    }

    /// Collections in `library`, or matching `search`, by name.
    pub(super) async fn collections(
        &self,
        library: Option<&LibraryId>,
        search: Option<&str>,
    ) -> Result<PageResult<SeriesCollection>, ApiError> {
        let sort = order(CollectionSort::Name, SortDirection::Asc);
        let libraries = library.map(std::slice::from_ref);
        self.state
            .collections
            .list(self.user, libraries, search, &sort, self.page)
            .await
            .map_err(|error| ApiError::from_collection(&error, self.path))
    }

    pub(super) async fn read_lists(
        &self,
        library: Option<&LibraryId>,
        search: Option<&str>,
    ) -> Result<PageResult<ReadList>, ApiError> {
        let sort = order(ReadListSort::Name, SortDirection::Asc);
        let libraries = library.map(std::slice::from_ref);
        self.state
            .read_lists
            .list(self.user, libraries, search, &sort, self.page)
            .await
            .map_err(|error| ApiError::from_read_list(&error, self.path))
    }
}
