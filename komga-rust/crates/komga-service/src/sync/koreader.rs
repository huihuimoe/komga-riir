//! Kotlin `KoreaderSyncController` and `KoreaderProgressConverter`: KOReader progress of a book
//! found by its KOReader hash, as a page for DiViNa and PDF books and as an EPUB resource
//! (`/body/DocFragment[n]`) otherwise.

use jiff::Timestamp;
use komga_db::{BookDao, Database, DbError, DbTransaction, MediaDao, ReadProgressDao};
use komga_types::{
    Book, KomgaUser, Media, MediaExtensionEpub, MediaProfile, R2Device, R2Location, R2Locator,
    R2Progression,
};
use thiserror::Error;
use tracing::error;

use crate::reading::progress::{ReadProgressError, ReadProgressService};
use crate::support::access_policy;

#[derive(Debug, Error)]
pub enum KoreaderError {
    #[error("Book not found")]
    NotFound,
    #[error("More than 1 book found with the same hash")]
    Conflict,
    #[error("Book has no media profile")]
    NoProfile,
    /// The reported progress does not point into the book.
    #[error("Unusable progress")]
    BadProgress,
    #[error(transparent)]
    Progress(#[from] ReadProgressError),
    #[error("Database error: {0}")]
    Db(#[from] DbError),
}

/// Kotlin `DocumentProgressDto` without its document hash.
#[derive(Debug, Clone, PartialEq)]
pub struct KoreaderProgress {
    /// Between 0 and 1.
    pub percentage: f32,
    /// A page number, or an XPointer/anchor into an EPUB resource.
    pub progress: String,
    pub device: R2Device,
}

#[derive(Clone)]
pub struct KoreaderSyncService {
    db: Database,
    read_progress: ReadProgressService,
}

impl KoreaderSyncService {
    #[must_use]
    pub fn new(db: Database, read_progress: ReadProgressService) -> Self {
        Self { db, read_progress }
    }

    /// The user's progress on the book with `hash`; `None` when they have none.
    pub async fn progress(
        &self,
        user: &KomgaUser,
        hash: &str,
    ) -> Result<Option<KoreaderProgress>, KoreaderError> {
        let mut tx = self.db.begin().await?;
        let (book, media) = book(&mut tx, user, hash).await?;
        let Some(saved) = ReadProgressDao::find(&mut tx, &book.id, &user.id).await? else {
            return Ok(None);
        };
        let locations = saved.locator.as_ref().and_then(|l| l.locations.as_ref());
        #[expect(clippy::cast_precision_loss, reason = "page numbers are small")]
        let percentage = locations
            .and_then(|locations| locations.total_progression)
            .unwrap_or_else(|| saved.page as f32 / media.page_count.max(1) as f32);
        let progress = match profile(&media)? {
            MediaProfile::Divina | MediaProfile::Pdf => saved.page.to_string(),
            MediaProfile::Epub => {
                let extension = extension(&mut tx, &book).await?;
                let href = saved
                    .locator
                    .as_ref()
                    .map(|locator| resource(&locator.href));
                // A progress without a resource points to the first one.
                let index = resources(&extension)
                    .iter()
                    .position(|resource| Some(*resource) == href)
                    .unwrap_or(0);
                format!("/body/DocFragment[{}].0", index + 1)
            }
        };
        tx.rollback().await?;
        Ok(Some(KoreaderProgress {
            percentage,
            progress,
            device: R2Device {
                id: saved.device_id,
                name: saved.device_name,
            },
        }))
    }

    /// `markProgression` at the page or at the start of the EPUB resource KOReader reports.
    pub async fn update_progress(
        &self,
        user: &KomgaUser,
        hash: &str,
        update: KoreaderProgress,
    ) -> Result<(), KoreaderError> {
        let mut tx = self.db.begin().await?;
        let (book, media) = book(&mut tx, user, hash).await?;
        // Kotlin reads a profileless book as 404 but refuses to write to it with 400.
        let locator = match profile(&media).map_err(|_| KoreaderError::BadProgress)? {
            MediaProfile::Divina | MediaProfile::Pdf => R2Locator {
                locations: Some(R2Location {
                    position: Some(
                        update
                            .progress
                            .parse()
                            .map_err(|_| KoreaderError::BadProgress)?,
                    ),
                    total_progression: Some(update.percentage),
                    ..R2Location::default()
                }),
                ..R2Locator::default()
            },
            MediaProfile::Epub => {
                let index = resource_index(&update.progress).ok_or(KoreaderError::BadProgress)?;
                let extension = extension(&mut tx, &book).await?;
                let href = resources(&extension)
                    .get(index)
                    .ok_or(KoreaderError::BadProgress)?
                    .to_string();
                R2Locator {
                    href,
                    media_type: "application/xhtml+xml".to_owned(),
                    locations: Some(R2Location {
                        progression: Some(0.0),
                        total_progression: Some(update.percentage),
                        ..R2Location::default()
                    }),
                    ..R2Locator::default()
                }
            }
        };
        tx.rollback().await?;
        let progression = R2Progression {
            modified: Timestamp::now(),
            device: update.device,
            locator,
        };
        Ok(self
            .read_progress
            .mark_progression(user, &book.id, progression)
            .await?)
    }
}

/// The only book with `hash` the user sees, and its media; books they cannot access do not
/// count, so another user's library neither answers nor conflicts.
async fn book(
    tx: &mut DbTransaction,
    user: &KomgaUser,
    hash: &str,
) -> Result<(Book, Media), KoreaderError> {
    if hash.is_empty() {
        return Err(KoreaderError::NotFound);
    }
    let mut visible = Vec::new();
    for book in BookDao::find_all_by_hash_koreader(tx, hash).await? {
        if access_policy::can_access_book(tx, user, &book).await? {
            visible.push(book);
        }
    }
    let book = match <[Book; 1]>::try_from(visible) {
        Ok([book]) => book,
        Err(books) if books.is_empty() => return Err(KoreaderError::NotFound),
        Err(_) => return Err(KoreaderError::Conflict),
    };
    let media = MediaDao::find_by_id(tx, &book.id)
        .await?
        .ok_or(KoreaderError::NoProfile)?;
    Ok((book, media))
}

fn profile(media: &Media) -> Result<MediaProfile, KoreaderError> {
    MediaProfile::from_media_type(media.media_type.as_deref()).ok_or(KoreaderError::NoProfile)
}

async fn extension(
    tx: &mut DbTransaction,
    book: &Book,
) -> Result<MediaExtensionEpub, KoreaderError> {
    MediaDao::find_extension(tx, &book.id)
        .await?
        .ok_or_else(|| {
            error!(book = %book.id, "Epub extension not found, the book should be re-analyzed");
            ReadProgressError::EpubExtensionNotFound.into()
        })
}

/// The EPUB resources with positions, in reading order: what KOReader's fragment index counts.
fn resources(extension: &MediaExtensionEpub) -> Vec<&str> {
    let mut resources: Vec<&str> = Vec::new();
    for position in &extension.positions {
        if !resources.contains(&position.href.as_str()) {
            resources.push(&position.href);
        }
    }
    resources
}

fn resource(href: &str) -> &str {
    href.split_once('#').map_or(href, |(path, _)| path)
}

/// `KoreaderUtils.parseResourceIndexFromProgressString`: the 0-based resource of
/// `/body/DocFragment[n]/…` (1-based), `…DocFragment/…` (the first), or `#_doc_fragment_n_…`
/// (0-based).
fn resource_index(progress: &str) -> Option<usize> {
    let progress = progress.to_ascii_lowercase();
    if let Some(number) = number_between(&progress, "docfragment[", ']') {
        return number.checked_sub(1);
    }
    if progress.contains("docfragment/") {
        return Some(0);
    }
    number_between(&progress, "#_doc_fragment_", '_')
}

/// The first number found right after a `prefix` and followed by `end`.
fn number_between(text: &str, prefix: &str, end: char) -> Option<usize> {
    text.match_indices(prefix).find_map(|(start, _)| {
        let rest = &text[start + prefix.len()..];
        let (digits, _) = rest.split_once(end)?;
        // `parse` alone would accept a leading `+`.
        if digits.bytes().all(|byte| byte.is_ascii_digit()) {
            digits.parse().ok()
        } else {
            None
        }
    })
}

#[cfg(test)]
mod tests {
    use super::resource_index;

    #[test]
    fn koreader_progress_strings_point_to_resources() {
        for (progress, index) in [
            ("/body/DocFragment[10]/body/div/p[1]/text().0", Some(9)),
            ("/body/docfragment[1].0", Some(0)),
            ("/body/DocFragment/body/p[3]/text().5", Some(0)),
            ("#_doc_fragment_44_ c37", Some(44)),
            ("/body/DocFragment[0].0", None),
            ("/body/DocFragment[x]/DocFragment[2].0", Some(1)),
            ("12", None),
        ] {
            assert_eq!(resource_index(progress), index, "{progress}");
        }
    }
}
