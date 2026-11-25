use komga_db::{BookDtoDao, ReadListDao, SeriesCollectionDao, SeriesDtoDao, Sibling};
use komga_types::{
    BookCondition, BookDetails, BookId, BookSort, CollectionId, ContentScope, Equality, KomgaUser,
    PageRequest, PageResult, ReadListId, SeriesCondition, SeriesDetails, SeriesSort, SortDirection,
    SortOrder,
};

use super::{CatalogError, CatalogService};

impl CatalogService {
    /// `getSeriesByCollectionId`: the series of a collection `user` may see that match all of
    /// `filters`; ordered collections by number, others by title sort.
    pub async fn collection_series(
        &self,
        user: &KomgaUser,
        id: &CollectionId,
        filters: Vec<SeriesCondition>,
        page: PageRequest,
    ) -> Result<PageResult<SeriesDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let collection = SeriesCollectionDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(CatalogError::NotFound)?;
        let mut conditions = vec![SeriesCondition::CollectionId(Equality::Is {
            value: id.as_str().to_owned(),
        })];
        conditions.extend(filters);
        let condition = SeriesCondition::AllOf(conditions);
        let sort = [SortOrder {
            key: if collection.ordered {
                SeriesSort::CollectionNumber
            } else {
                SeriesSort::TitleSort
            },
            direction: SortDirection::Asc,
        }];
        let series = SeriesDtoDao::find_all(
            &mut tx,
            &user.id,
            &scope,
            Some(&condition),
            None,
            &sort,
            page,
        )
        .await?;
        tx.rollback().await?;
        Ok(series)
    }

    /// `getBooksByReadListId`: the books of a read list `user` may see that match all of
    /// `filters`, in the order of [`read_list_order`].
    pub async fn read_list_books(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
        filters: Vec<BookCondition>,
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let read_list = ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(CatalogError::NotFound)?;
        let mut conditions = vec![in_read_list(id)];
        conditions.extend(filters);
        let condition = BookCondition::AllOf(conditions);
        let sort = read_list_order(read_list.ordered);
        let books = BookDtoDao::find_all(
            &mut tx,
            &user.id,
            &scope,
            Some(&condition),
            None,
            &sort,
            page,
        )
        .await?;
        tx.rollback().await?;
        Ok(books)
    }

    /// `getBookSiblingPreviousInReadList`/`getBookSiblingNextInReadList`: the neighbour of a book
    /// among the books of a read list `user` may see, in the order of [`read_list_order`].
    pub async fn read_list_book_sibling(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
        book_id: &BookId,
        sibling: Sibling,
    ) -> Result<BookDetails, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let read_list = ReadListDao::find_by_id(&mut tx, id, Some(&scope))
            .await?
            .ok_or(CatalogError::NotFound)?;
        let sort = read_list_order(read_list.ordered);
        let found = BookDtoDao::find_sibling(
            &mut tx,
            &user.id,
            &scope,
            &in_read_list(id),
            &sort,
            book_id,
            sibling,
        )
        .await?;
        tx.rollback().await?;
        found.ok_or(CatalogError::NotFound)
    }
}

fn in_read_list(id: &ReadListId) -> BookCondition {
    BookCondition::ReadListId(Equality::Is {
        value: id.as_str().to_owned(),
    })
}

/// Ordered read lists by number, others by release date; the number then orders books of the
/// same date, where Kotlin leaves their order to the database.
fn read_list_order(ordered: bool) -> Vec<SortOrder<BookSort>> {
    let ascending = |key| SortOrder {
        key,
        direction: SortDirection::Asc,
    };
    if ordered {
        vec![ascending(BookSort::ReadListNumber)]
    } else {
        vec![
            ascending(BookSort::ReleaseDate),
            ascending(BookSort::ReadListNumber),
        ]
    }
}
