//! Kotlin metadata providers, in the order Spring injects them: the component scan order of
//! their packages.

mod barcode;
pub(crate) mod comic_info;
mod comic_rack;
mod epub;
mod mylar;

use komga_types::{
    Book, BookMetadata, BookMetadataPatchCapability, Library, Media, MediaProfile, MediaStatus,
    Series, SeriesStatus,
};
use tokio::fs;
use tracing::error;

use super::edit::patch::BookMetadataPatch;
use super::edit::series_patch::SeriesMetadataPatch;
use crate::reading::ReadingService;

/// Kotlin `MetadataPatchTarget`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum Target {
    Book,
    ReadList,
    Series,
    Collection,
}

/// `ComicInfoProvider.shouldLibraryHandlePatch`.
fn comic_info_handles(library: &Library, target: Target) -> bool {
    match target {
        Target::Book => library.import_comic_info_book,
        Target::ReadList => library.import_comic_info_read_list,
        Target::Series => library.import_comic_info_series,
        Target::Collection => library.import_comic_info_collection,
    }
}

/// `EpubMetadataProvider.shouldLibraryHandlePatch`.
fn epub_handles(library: &Library, target: Target) -> bool {
    match target {
        Target::Book => library.import_epub_book,
        Target::Series => library.import_epub_series,
        Target::ReadList | Target::Collection => false,
    }
}

fn is_epub_profile(media: &Media) -> bool {
    MediaProfile::from_media_type(media.media_type.as_deref()) == Some(MediaProfile::Epub)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum BookProvider {
    IsbnBarcode,
    ComicInfo,
    Epub,
}

pub(super) const BOOK_PROVIDERS: [BookProvider; 3] = [
    BookProvider::IsbnBarcode,
    BookProvider::ComicInfo,
    BookProvider::Epub,
];

impl BookProvider {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::IsbnBarcode => "IsbnBarcodeProvider",
            Self::ComicInfo => "ComicInfoProvider",
            Self::Epub => "EpubMetadataProvider",
        }
    }

    pub(super) fn capabilities(self) -> &'static [BookMetadataPatchCapability] {
        match self {
            Self::IsbnBarcode => &[BookMetadataPatchCapability::Isbn],
            Self::ComicInfo => &[
                BookMetadataPatchCapability::Title,
                BookMetadataPatchCapability::Summary,
                BookMetadataPatchCapability::Number,
                BookMetadataPatchCapability::NumberSort,
                BookMetadataPatchCapability::ReleaseDate,
                BookMetadataPatchCapability::Authors,
                BookMetadataPatchCapability::NotableEntities,
                BookMetadataPatchCapability::ReadLists,
                BookMetadataPatchCapability::Links,
            ],
            Self::Epub => &[
                BookMetadataPatchCapability::Title,
                BookMetadataPatchCapability::Summary,
                BookMetadataPatchCapability::ReleaseDate,
                BookMetadataPatchCapability::Authors,
                BookMetadataPatchCapability::Isbn,
            ],
        }
    }

    /// `shouldLibraryHandlePatch`.
    pub(super) fn handles(self, library: &Library, target: Target) -> bool {
        match self {
            Self::IsbnBarcode => target == Target::Book && library.import_barcode_isbn,
            Self::ComicInfo => comic_info_handles(library, target),
            Self::Epub => epub_handles(library, target),
        }
    }

    /// `getBookMetadataFromBook`; `None` when the provider finds nothing or fails, which it logs.
    pub(super) async fn book_patch(
        self,
        reading: &ReadingService,
        book: &Book,
        media: &Media,
    ) -> Option<BookMetadataPatch> {
        match self {
            Self::IsbnBarcode => {
                if is_epub_profile(media) {
                    return None;
                }
                // Each page read fails for media that is not ready, which finds nothing.
                if media.status != MediaStatus::Ready || media.page_count == 0 {
                    return None;
                }
                barcode::book_patch(reading, book, media.page_count).await
            }
            Self::ComicInfo => comic_rack::book_patch(book, media).await,
            Self::Epub => {
                if !is_epub_profile(media) {
                    return None;
                }
                epub::book_patch(book, media).await
            }
        }
    }
}

/// Kotlin `SeriesMetadataFromBookProvider`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SeriesFromBookProvider {
    ComicInfo,
    Epub,
}

pub(super) const SERIES_FROM_BOOK_PROVIDERS: [SeriesFromBookProvider; 2] = [
    SeriesFromBookProvider::ComicInfo,
    SeriesFromBookProvider::Epub,
];

impl SeriesFromBookProvider {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::ComicInfo => "ComicInfoProvider",
            Self::Epub => "EpubMetadataProvider",
        }
    }

    /// `shouldLibraryHandlePatch`.
    pub(super) fn handles(self, library: &Library, target: Target) -> bool {
        match self {
            Self::ComicInfo => comic_info_handles(library, target),
            Self::Epub => epub_handles(library, target),
        }
    }

    /// `getSeriesMetadataFromBook`; `None` when the provider finds nothing or fails, which it
    /// logs.
    pub(super) async fn series_patch(
        self,
        book: &Book,
        media: &Media,
        append_volume: bool,
    ) -> Option<SeriesMetadataPatch> {
        match self {
            Self::ComicInfo => comic_rack::series_patch(book, media, append_volume).await,
            Self::Epub => {
                if !is_epub_profile(media) {
                    return None;
                }
                epub::series_patch(book, media).await
            }
        }
    }
}

/// Kotlin `SeriesMetadataProvider`s.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(super) enum SeriesProvider {
    Mylar,
    OneShot,
}

pub(super) const SERIES_PROVIDERS: [SeriesProvider; 2] =
    [SeriesProvider::Mylar, SeriesProvider::OneShot];

impl SeriesProvider {
    pub(super) fn name(self) -> &'static str {
        match self {
            Self::Mylar => "MylarSeriesProvider",
            Self::OneShot => "OneShotSeriesProvider",
        }
    }

    /// `shouldLibraryHandlePatch` for `SERIES`, the only target of these providers.
    pub(super) fn handles_series(self, library: &Library) -> bool {
        match self {
            Self::Mylar => library.import_mylar_series,
            Self::OneShot => true,
        }
    }

    /// `getSeriesMetadata`; `None` when the provider finds nothing. `first_book` is the metadata
    /// of the series' first book, read only for oneshots.
    pub(super) async fn series_patch(
        self,
        series: &Series,
        first_book: Option<&BookMetadata>,
    ) -> Option<SeriesMetadataPatch> {
        match self {
            Self::Mylar => {
                if series.oneshot {
                    return None;
                }
                let path = series.url.to_path()?.join(mylar::SERIES_JSON);
                if !fs::try_exists(&path).await.unwrap_or(false) {
                    return None;
                }
                mylar::series_patch(&path).await
            }
            Self::OneShot => {
                if !series.oneshot {
                    return None;
                }
                let Some(book) = first_book else {
                    error!(series = %series.id, "Error while getting metadata from OneShotSeriesProvider: the oneshot has no book");
                    return None;
                };
                Some(SeriesMetadataPatch {
                    title: Some(book.title.clone()),
                    title_sort: Some(book.title.clone()),
                    status: Some(SeriesStatus::Ended),
                    summary: Some(book.summary.clone()),
                    total_book_count: Some(1),
                    ..SeriesMetadataPatch::default()
                })
            }
        }
    }
}
