use komga_db::{BookDtoDao, Sibling};
use komga_types::{
    BookCondition, BookDetails, BookId, BookSort, ContentScope, Equality, KomgaUser, LibraryId,
    PageRequest, PageResult, SearchEntity, SeriesId, SortOrder,
};

use super::{CatalogError, CatalogService};
use crate::support::access_policy::{self, Access};

impl CatalogService {
    /// `getBookById`: the book as `user` sees it.
    pub async fn book(&self, user: &KomgaUser, id: &BookId) -> Result<BookDetails, CatalogError> {
        let mut tx = self.db.begin().await?;
        let book = BookDtoDao::find_by_id(&mut tx, id, &user.id)
            .await?
            .ok_or(CatalogError::NotFound)?;
        if !access_policy::can_access_book(&mut tx, user, &book.book).await? {
            return Err(CatalogError::Restricted);
        }
        tx.rollback().await?;
        Ok(book)
    }

    /// `getBookSiblingPrevious`/`getBookSiblingNext`: the neighbour of a book `user` may see in
    /// its series, which they may then see as well.
    pub async fn book_sibling(
        &self,
        user: &KomgaUser,
        id: &BookId,
        sibling: Sibling,
    ) -> Result<BookDetails, CatalogError> {
        let mut tx = self.db.begin().await?;
        match access_policy::check_book_id(&mut tx, user, id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(CatalogError::Restricted),
            Access::Unknown => return Err(CatalogError::NotFound),
        }
        let found = BookDtoDao::find_sibling_in_series(&mut tx, id, &user.id, sibling).await?;
        tx.rollback().await?;
        found.ok_or(CatalogError::NotFound)
    }

    /// `findAll(BookSearch, SearchContext(user), pageable)`, with the full-text search
    /// `full_text` when given, which is not blank.
    pub async fn books(
        &self,
        user: &KomgaUser,
        condition: Option<&BookCondition>,
        full_text: Option<&str>,
        sort: &[SortOrder<BookSort>],
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut search = self
            .full_text
            .begin(&self.db, SearchEntity::Book, full_text)
            .await?;
        let books = BookDtoDao::find_all(
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
        Ok(books)
    }

    /// `getBooksBySeriesId`: the books of a series `user` may see that match all of `filters`.
    pub async fn series_books(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
        filters: Vec<BookCondition>,
        sort: &[SortOrder<BookSort>],
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        match access_policy::check_series_id(&mut tx, user, id).await? {
            Access::Allowed => {}
            Access::Denied => return Err(CatalogError::Restricted),
            Access::Unknown => return Err(CatalogError::NotFound),
        }
        let mut conditions = vec![BookCondition::SeriesId(Equality::Is {
            value: id.as_str().to_owned(),
        })];
        conditions.extend(filters);
        let condition = BookCondition::AllOf(conditions);
        let books = BookDtoDao::find_all(
            &mut tx,
            &user.id,
            &scope,
            Some(&condition),
            None,
            sort,
            page,
        )
        .await?;
        tx.rollback().await?;
        Ok(books)
    }

    /// `findAllDuplicates`: the visible books sharing their file hash and size with another book.
    pub async fn duplicate_books(
        &self,
        user: &KomgaUser,
        sort: &[SortOrder<BookSort>],
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let books = BookDtoDao::find_duplicates(&mut tx, &user.id, &scope, sort, page).await?;
        tx.rollback().await?;
        Ok(books)
    }

    /// `findAllOnDeck`: the next book to read in each series the user has started and paused,
    /// within their visible content and `libraries` when given.
    pub async fn books_on_deck(
        &self,
        user: &KomgaUser,
        libraries: Option<&[LibraryId]>,
        page: PageRequest,
    ) -> Result<PageResult<BookDetails>, CatalogError> {
        let scope = ContentScope::for_user(user);
        let mut tx = self.db.begin().await?;
        let books = BookDtoDao::find_on_deck(&mut tx, &user.id, &scope, libraries, page).await?;
        tx.commit().await?;
        Ok(books)
    }
}
