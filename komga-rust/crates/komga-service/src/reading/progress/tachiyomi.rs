//! The Mihon (Tachiyomi) read progress of `SeriesController` and `ReadListController`, which
//! counts the books a user read and marks books read up to a point.

use jiff::Timestamp;
use komga_db::{
    DbTransaction, NumberedCompletion, ReadListDao, ReadProgressDao, ReadProgressDtoDao, SeriesDao,
};
use komga_types::{ContentScope, DomainEvent, KomgaUser, ReadList, ReadListId, SeriesId};

use super::{ReadProgressError, ReadProgressService, record_books, series};
use crate::support::access_policy;

/// Kotlin `BooksCount`.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ReadCounts {
    pub read: i32,
    pub unread: i32,
    pub in_progress: i32,
}

impl ReadCounts {
    #[must_use]
    pub fn total(self) -> i32 {
        self.read + self.unread + self.in_progress
    }

    fn of(books: impl IntoIterator<Item = Option<bool>>) -> Self {
        let mut counts = Self::default();
        for completed in books {
            match completed {
                Some(true) => counts.read += 1,
                Some(false) => counts.in_progress += 1,
                None => counts.unread += 1,
            }
        }
        counts
    }
}

/// Kotlin `TachiyomiReadProgressV2Dto`.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct SeriesTachiyomiProgress {
    pub counts: ReadCounts,
    /// The `numberSort` of the last book of the leading run of completed books, else 0.
    pub last_read_continuous_number_sort: f32,
    /// 0 without books.
    pub max_number_sort: f32,
}

/// Kotlin `TachiyomiReadProgressDto`, over the books of a read list the user sees.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ReadListTachiyomiProgress {
    pub counts: ReadCounts,
    /// How many books lead the list completed.
    pub last_read_continuous_index: i32,
}

impl ReadProgressService {
    /// `findProgressV2BySeries` of a series the user sees, soft-deleted books included.
    pub async fn series_tachiyomi(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
    ) -> Result<SeriesTachiyomiProgress, ReadProgressError> {
        let mut tx = self.db.begin().await?;
        // Kotlin fails on the counts of an unknown series; it is a 404 for every user here.
        let series = SeriesDao::find_by_id(&mut tx, id)
            .await?
            .ok_or(ReadProgressError::NotFound)?;
        if !access_policy::can_access_series(&mut tx, user, &series).await? {
            return Err(ReadProgressError::Restricted);
        }
        let books = ReadProgressDtoDao::find_series_completion(&mut tx, id, &user.id).await?;
        tx.commit().await?;
        Ok(series_progress(&books))
    }

    /// `updateMihonReadProgressBySeriesId`: completes the books of the series whose `numberSort`
    /// is not above `up_to_number_sort`.
    pub async fn mark_series_tachiyomi(
        &self,
        user: &KomgaUser,
        id: &SeriesId,
        up_to_number_sort: f32,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        series::check(access_policy::check_series_id(&mut tx, user, id).await?)?;
        let completed = ReadProgressDao::complete_series(
            &mut tx,
            id,
            Some(up_to_number_sort),
            &user.id,
            Timestamp::now(),
        )
        .await?;
        record_books(&mut tx, completed, user, DomainEvent::ReadProgressChanged);
        self.events.commit(tx).await?;
        Ok(())
    }

    /// `findProgressByReadList`. Kotlin counts the books hidden from the user too, so its index
    /// would not match the visible books the update counts; only visible books count here.
    pub async fn read_list_tachiyomi(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
    ) -> Result<ReadListTachiyomiProgress, ReadProgressError> {
        let mut tx = self.db.begin().await?;
        let read_list = visible_read_list(&mut tx, user, id).await?;
        let books =
            ReadProgressDtoDao::find_completion(&mut tx, &read_list.book_ids, &user.id).await?;
        tx.commit().await?;
        let continuous = books
            .iter()
            .take_while(|&&completed| completed == Some(true))
            .count();
        Ok(ReadListTachiyomiProgress {
            counts: ReadCounts::of(books.iter().copied()),
            last_read_continuous_index: i32::try_from(continuous)
                .expect("A read list holds fewer books than i32::MAX"),
        })
    }

    /// `updateMihonReadProgressByReadListId`: completes the first `last_book_read` books of the
    /// read list the user sees.
    pub async fn mark_read_list_tachiyomi(
        &self,
        user: &KomgaUser,
        id: &ReadListId,
        last_book_read: usize,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        let mut books = visible_read_list(&mut tx, user, id).await?.book_ids;
        books.truncate(last_book_read);
        let completed =
            ReadProgressDao::complete_books(&mut tx, &books, &user.id, Timestamp::now()).await?;
        record_books(&mut tx, completed, user, DomainEvent::ReadProgressChanged);
        self.events.commit(tx).await?;
        Ok(())
    }
}

async fn visible_read_list(
    transaction: &mut DbTransaction,
    user: &KomgaUser,
    id: &ReadListId,
) -> Result<ReadList, ReadProgressError> {
    let scope = ContentScope::for_user(user);
    ReadListDao::find_by_id(transaction, id, Some(&scope))
        .await?
        .ok_or(ReadProgressError::NotFound)
}

fn series_progress(books: &[NumberedCompletion]) -> SeriesTachiyomiProgress {
    let counts = ReadCounts::of(books.iter().map(|book| book.completed));
    let last_read_continuous_number_sort = books
        .iter()
        .take_while(|book| book.completed == Some(true))
        .last()
        .map_or(0.0, |book| book.number_sort);
    let max_number_sort = books
        .iter()
        .map(|book| book.number_sort)
        .reduce(f32::max)
        .unwrap_or(0.0);
    SeriesTachiyomiProgress {
        counts,
        last_read_continuous_number_sort,
        max_number_sort,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn book(number_sort: f32, completed: Option<bool>) -> NumberedCompletion {
        NumberedCompletion {
            number_sort,
            completed,
        }
    }

    #[test]
    fn series_progress_stops_the_continuous_run_at_the_first_book_not_completed() {
        let books = [
            book(1.0, Some(true)),
            book(2.5, Some(true)),
            book(3.0, Some(false)),
            book(4.0, Some(true)),
            book(5.0, None),
        ];

        assert_eq!(
            series_progress(&books),
            SeriesTachiyomiProgress {
                counts: ReadCounts {
                    read: 3,
                    unread: 1,
                    in_progress: 1,
                },
                last_read_continuous_number_sort: 2.5,
                max_number_sort: 5.0,
            }
        );
    }

    #[test]
    fn series_progress_is_zero_without_a_leading_completed_book() {
        let progress = series_progress(&[book(1.0, None), book(2.0, Some(true))]);

        assert_eq!(progress.last_read_continuous_number_sort, 0.0);
        assert_eq!(progress.max_number_sort, 2.0);
        assert_eq!(series_progress(&[]).max_number_sort, 0.0);
    }
}
