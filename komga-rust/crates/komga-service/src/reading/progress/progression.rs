//! Kotlin `CommonBookController.getBookProgression` and `BookLifecycle.markProgression`: Readium
//! progressions, as page positions for DiViNa and PDF books and as EPUB locators otherwise.

use komga_archive::epub;
use komga_db::{MediaDao, ReadProgressDao};
use komga_types::{
    BookId, KomgaUser, Media, MediaExtensionEpub, MediaProfile, R2Locator, R2Progression,
    ReadProgress,
};
use tracing::error;

use super::{ReadProgressError, ReadProgressService, media, visible_book};

impl ReadProgressService {
    /// The user's progress on a book they see.
    pub async fn find(
        &self,
        user: &KomgaUser,
        id: &BookId,
    ) -> Result<Option<ReadProgress>, ReadProgressError> {
        let mut tx = self.db.begin().await?;
        let book = visible_book(&mut tx, user, id).await?;
        let progress = ReadProgressDao::find(&mut tx, &book.id, &user.id).await?;
        tx.rollback().await?;
        Ok(progress)
    }

    /// `markProgression`: replaces the user's progress unless it was read at or after
    /// `progression.modified`.
    pub async fn mark_progression(
        &self,
        user: &KomgaUser,
        id: &BookId,
        progression: R2Progression,
    ) -> Result<(), ReadProgressError> {
        let mut tx = self.db.begin_immediate().await?;
        let book = visible_book(&mut tx, user, id).await?;
        let saved = ReadProgressDao::find(&mut tx, &book.id, &user.id).await?;
        if saved.is_some_and(|saved| progression.modified <= saved.read_date) {
            return Err(ReadProgressError::OutdatedProgression);
        }
        let media = media(&mut tx, &book.id).await?;
        let (page, completed, locator) = match MediaProfile::from_media_type(
            media.media_type.as_deref(),
        ) {
            None => return Err(ReadProgressError::NoProfile),
            Some(MediaProfile::Divina | MediaProfile::Pdf) => {
                let position = progression
                    .locator
                    .locations
                    .as_ref()
                    .and_then(|locations| locations.position);
                let page = position
                    .filter(|position| (1..=media.page_count).contains(position))
                    .ok_or(ReadProgressError::PageOutOfRange {
                        page: position,
                        page_count: media.page_count,
                    })?;
                (page, page == media.page_count, progression.locator)
            }
            Some(MediaProfile::Epub) => {
                let Some(extension) = MediaDao::find_extension(&mut tx, &book.id).await? else {
                    error!(book = %book.id, "Epub extension not found, the book should be re-analyzed");
                    return Err(ReadProgressError::EpubExtensionNotFound);
                };
                epub_progress(&media, &extension, progression.locator)?
            }
        };
        let progress = ReadProgress {
            book_id: book.id,
            user_id: user.id.clone(),
            page,
            completed,
            read_date: progression.modified,
            device_id: progression.device.id,
            device_name: progression.device.name,
            locator: Some(locator),
        };
        self.save(tx, &progress).await
    }
}

/// The page, completion, and locator of an EPUB progression: the analyzed position at the
/// locator's progression in its resource, else the closest one before it, gives the total
/// progression; the reported total progression is not trusted.
fn epub_progress(
    media: &Media,
    extension: &MediaExtensionEpub,
    mut locator: R2Locator,
) -> Result<(i32, bool, R2Locator), ReadProgressError> {
    let resource = locator
        .href
        .split_once('#')
        .map_or(&*locator.href, |(path, _)| path);
    let href = epub::url_decode(resource)?;
    if !media.files.iter().any(|file| file.file_name == href) {
        return Err(ReadProgressError::UnknownResource(href));
    }
    let Some(locations) = locator.locations.as_mut() else {
        return Err(ReadProgressError::ProgressionRequired);
    };
    let progression = locations
        .progression
        .ok_or(ReadProgressError::ProgressionRequired)?;

    let matching: Vec<&R2Locator> = extension
        .positions
        .iter()
        .filter(|position| position.href == href)
        .collect();
    let matched = match matching.as_slice() {
        [only] if extension.is_fixed_layout => *only,
        _ => matching_position(&matching, progression)
            .ok_or(ReadProgressError::InvalidProgression)?,
    };

    let total = matched
        .locations
        .as_ref()
        .and_then(|locations| locations.total_progression);
    locations.total_progression = total;
    locator.media_type.clone_from(&matched.media_type);
    if locator.kobo_span.is_none() {
        locator.kobo_span.clone_from(&matched.kobo_span);
    }
    #[expect(
        clippy::cast_possible_truncation,
        reason = "page count times progression in [0, 1] fits in i32"
    )]
    let page = total.map_or(0, |total| {
        (f64::from(media.page_count) * f64::from(total)).round() as i32
    });
    Ok((page, total.is_some_and(|total| total >= 0.99), locator))
}

/// The position at `progression`, else the last one before it when one comes after.
fn matching_position<'a>(positions: &[&'a R2Locator], progression: f32) -> Option<&'a R2Locator> {
    let at = |position: &R2Locator| position.locations.as_ref().and_then(|l| l.progression);
    let number = |position: &&R2Locator| position.locations.as_ref().and_then(|l| l.position);
    let positions = positions.iter().copied();
    if let Some(exact) = positions
        .clone()
        .find(|position| at(position) == Some(progression))
    {
        return Some(exact);
    }
    let before = positions
        .clone()
        .filter(|position| at(position).is_some_and(|at| at < progression))
        .max_by_key(number)?;
    let after = positions
        .filter(|position| at(position).is_some_and(|at| at > progression))
        .min_by_key(number)?;
    (number(&before) <= number(&after)).then_some(before)
}
