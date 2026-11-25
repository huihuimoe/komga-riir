//! How the errors of `komga-service` use cases answer HTTP requests.

use axum::http::StatusCode;
use komga_service::{
    ArtworkError, BlockingError, BookAnalysisError, CatalogError, CollectionError, FilesystemError,
    FontError, IdentityError, LibraryError, MetadataError, PageHashError, ReadListError,
    ReadProgressError, ReadingError, SearchBudgetError, SearchIndexError, TaskError,
    TransientBookError,
};
use komga_types::MediaStatus;

use super::ApiError;

impl ApiError {
    pub(crate) fn from_identity(error: IdentityError, path: &str) -> Self {
        match error {
            IdentityError::AlreadyClaimed => Self::new(
                StatusCode::BAD_REQUEST,
                "This server has already been claimed",
                path,
            ),
            IdentityError::UserAlreadyExists(_) => Self::new(
                StatusCode::BAD_REQUEST,
                "A user with this email already exists",
                path,
            ),
            IdentityError::ApiKeyCommentDuplicate => {
                Self::new(StatusCode::BAD_REQUEST, "ERR_1034", path)
            }
            IdentityError::UserNotFound => Self::new(StatusCode::NOT_FOUND, "User not found", path),
            IdentityError::ApiKeyNotFound => {
                Self::new(StatusCode::NOT_FOUND, "API key not found", path)
            }
            IdentityError::CannotDeleteSelf => Self::new(StatusCode::FORBIDDEN, "Forbidden", path),
            IdentityError::Db(_) | IdentityError::PasswordHash(_) | IdentityError::Worker(_) => {
                Self::internal_error(path, error)
            }
        }
    }

    pub(crate) fn from_library(error: LibraryError, path: &str) -> Self {
        match error {
            LibraryError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            LibraryError::Forbidden => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            LibraryError::Invalid(message) => Self::new(StatusCode::BAD_REQUEST, message, path),
            LibraryError::Blocking(error) => Self::from_blocking(&error, path),
            LibraryError::Db(_) | LibraryError::Io(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_catalog(error: &CatalogError, path: &str) -> Self {
        match error {
            CatalogError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            CatalogError::Restricted => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            CatalogError::Db(_) => Self::internal_error(path, error),
            CatalogError::Search(error) => Self::from_search(error, path),
        }
    }

    pub(crate) fn from_collection(error: &CollectionError, path: &str) -> Self {
        match error {
            CollectionError::NotFound | CollectionError::SeriesNotFound => {
                Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path)
            }
            CollectionError::Restricted => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            CollectionError::DuplicateName | CollectionError::UnknownSeries => {
                Self::new(StatusCode::BAD_REQUEST, error.to_string(), path)
            }
            CollectionError::Db(_) => Self::internal_error(path, error),
            CollectionError::Search(error) => Self::from_search(error, path),
        }
    }

    pub(crate) fn from_read_list(error: &ReadListError, path: &str) -> Self {
        match error {
            ReadListError::NotFound | ReadListError::BookNotFound => {
                Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path)
            }
            ReadListError::Forbidden => Self::forbidden(path),
            ReadListError::Restricted => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            ReadListError::DuplicateName | ReadListError::UnknownBook => {
                Self::new(StatusCode::BAD_REQUEST, error.to_string(), path)
            }
            ReadListError::ComicRackList(list_error) => {
                Self::new(StatusCode::BAD_REQUEST, list_error.code(), path)
            }
            ReadListError::Db(_) => Self::internal_error(path, error),
            ReadListError::Search(error) => Self::from_search(error, path),
        }
    }

    pub(crate) fn from_book_analysis(error: &BookAnalysisError, path: &str) -> Self {
        match error {
            BookAnalysisError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            BookAnalysisError::Forbidden => Self::forbidden(path),
            BookAnalysisError::Db(_)
            | BookAnalysisError::Worker(_)
            | BookAnalysisError::InvalidUrl(_)
            | BookAnalysisError::MissingMedia(_)
            | BookAnalysisError::MissingCodec(_)
            | BookAnalysisError::Io(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_metadata(error: &MetadataError, path: &str) -> Self {
        match error {
            MetadataError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            MetadataError::Forbidden => Self::forbidden(path),
            MetadataError::Db(_) | MetadataError::MissingMedia(_) => {
                Self::internal_error(path, error)
            }
        }
    }

    pub(crate) fn from_reading(error: &ReadingError, path: &str) -> Self {
        let not_found = |message| Self::new(StatusCode::NOT_FOUND, message, path);
        let bad_request = |message| Self::new(StatusCode::BAD_REQUEST, message, path);
        match error {
            ReadingError::NotFound | ReadingError::BookFileNotFound => {
                not_found("404 NOT_FOUND".to_owned())
            }
            ReadingError::Forbidden => Self::forbidden(path),
            ReadingError::Restricted => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            ReadingError::Unavailable(status) => not_found(
                match status {
                    MediaStatus::Unknown => "Book has not been analyzed yet",
                    MediaStatus::Outdated => "Book is outdated and must be re-analyzed",
                    MediaStatus::Error => "Book analysis failed",
                    MediaStatus::Unsupported => "Book format is not supported",
                    MediaStatus::Ready => unreachable!("Ready books list their pages"),
                }
                .to_owned(),
            ),
            ReadingError::MediaNotReady
            | ReadingError::FileNotFound
            | ReadingError::UnreadableFormat { .. } => not_found(error.to_string()),
            ReadingError::PageOutOfBounds
            | ReadingError::InvalidConversion(_)
            | ReadingError::RawUnsupported
            | ReadingError::IncompatibleProfile(_) => bad_request(error.to_string()),
            ReadingError::Blocking(error) => Self::from_blocking(error, path),
            ReadingError::Kepub(_) => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "Kepub conversion failed",
                path,
            ),
            ReadingError::Db(_)
            | ReadingError::InvalidUrl(_)
            | ReadingError::MissingMedia(_)
            | ReadingError::EpubPagesUnsupported
            | ReadingError::FilesNotReady
            | ReadingError::Archive(_)
            | ReadingError::Mobi(_)
            | ReadingError::Pdf(_)
            | ReadingError::Conversion(_)
            | ReadingError::Io(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_read_progress(error: &ReadProgressError, path: &str) -> Self {
        match error {
            ReadProgressError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            ReadProgressError::Restricted => {
                Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path)
            }
            ReadProgressError::OutdatedProgression => {
                Self::new(StatusCode::CONFLICT, error.to_string(), path)
            }
            ReadProgressError::PageOutOfRange { .. }
            | ReadProgressError::NotDivinaCompatible
            | ReadProgressError::EpubExtensionNotFound
            | ReadProgressError::NoProfile
            | ReadProgressError::MalformedHref(_)
            | ReadProgressError::UnknownResource(_)
            | ReadProgressError::ProgressionRequired
            | ReadProgressError::InvalidProgression => {
                Self::new(StatusCode::BAD_REQUEST, error.to_string(), path)
            }
            ReadProgressError::MissingPosition(..)
            | ReadProgressError::MissingMedia(_)
            | ReadProgressError::Db(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_font(error: &FontError, path: &str) -> Self {
        match error {
            FontError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            FontError::Read(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_artwork(error: &ArtworkError, path: &str) -> Self {
        match error {
            ArtworkError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            ArtworkError::Restricted => Self::new(StatusCode::FORBIDDEN, "403 FORBIDDEN", path),
            ArtworkError::OtherOwner | ArtworkError::Oneshot => {
                Self::new(StatusCode::BAD_REQUEST, "400 BAD_REQUEST", path)
            }
            ArtworkError::NotImage => Self::new(
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                "415 UNSUPPORTED_MEDIA_TYPE",
                path,
            ),
            ArtworkError::NotUploaded => Self::new(
                StatusCode::BAD_REQUEST,
                "Only uploaded thumbnails can be deleted",
                path,
            ),
            ArtworkError::Db(_)
            | ArtworkError::Settings(_)
            | ArtworkError::Worker(_)
            | ArtworkError::InvalidUrl(_)
            | ArtworkError::MissingMedia(_)
            | ArtworkError::NoCover(_)
            | ArtworkError::Archive(_)
            | ArtworkError::Epub(_)
            | ArtworkError::Mobi(_)
            | ArtworkError::Pdf(_)
            | ArtworkError::Image(_)
            | ArtworkError::Io(_) => Self::internal_error(path, error),
            ArtworkError::Blocking(error) => Self::from_blocking(error, path),
        }
    }

    pub(crate) fn from_filesystem(error: FilesystemError, path: &str) -> Self {
        match error {
            FilesystemError::Forbidden => Self::forbidden(path),
            FilesystemError::RelativePath => {
                Self::new(StatusCode::BAD_REQUEST, "Path must be absolute", path)
            }
            FilesystemError::Io(_) => {
                Self::new(StatusCode::BAD_REQUEST, "Path does not exist", path)
            }
            FilesystemError::Blocking(error) => Self::from_blocking(&error, path),
            FilesystemError::InvalidPath => Self::internal_error(path, error),
        }
    }

    /// Kotlin answers coded errors with their code.
    pub(crate) fn from_transient_book(error: &TransientBookError, path: &str) -> Self {
        match error {
            TransientBookError::Forbidden => Self::forbidden(path),
            TransientBookError::Scan(_) => Self::new(StatusCode::BAD_REQUEST, "ERR_1016", path),
            TransientBookError::InLibrary => Self::new(StatusCode::BAD_REQUEST, "ERR_1017", path),
            TransientBookError::NotFound => Self::new(StatusCode::NOT_FOUND, "404 NOT_FOUND", path),
            TransientBookError::Blocking(error) => Self::from_blocking(error, path),
            TransientBookError::Analysis(error) => Self::from_book_analysis(error, path),
            TransientBookError::Db(_) => Self::internal_error(path, error),
        }
    }

    pub(crate) fn from_task(error: &TaskError, path: &str) -> Self {
        match error {
            TaskError::Forbidden => Self::forbidden(path),
            TaskError::Db(_) => Self::internal_error(path, error),
        }
    }

    /// A page that cannot be read is a server failure: its hash came from reading it.
    pub(crate) fn from_page_hash(error: &PageHashError, path: &str) -> Self {
        match error {
            PageHashError::Reading(ReadingError::Blocking(error)) => {
                Self::from_blocking(error, path)
            }
            PageHashError::Db(_) | PageHashError::Reading(_) => Self::internal_error(path, error),
        }
    }

    /// A search over its budget says so, rather than answering with some of its hits.
    fn from_search(error: &SearchIndexError, path: &str) -> Self {
        match error {
            SearchIndexError::OverBudget(SearchBudgetError::TooManyHits { .. }) => {
                Self::new(StatusCode::BAD_REQUEST, error.to_string(), path)
            }
            SearchIndexError::OverBudget(SearchBudgetError::Busy | SearchBudgetError::TimedOut) => {
                Self::new(StatusCode::SERVICE_UNAVAILABLE, error.to_string(), path)
            }
            SearchIndexError::Db(_)
            | SearchIndexError::Index(_)
            | SearchIndexError::Panicked(_)
            | SearchIndexError::Interrupted => Self::internal_error(path, error),
        }
    }

    fn from_blocking(error: &BlockingError, path: &str) -> Self {
        match error {
            BlockingError::Saturated => Self::new(
                StatusCode::SERVICE_UNAVAILABLE,
                "503 SERVICE_UNAVAILABLE",
                path,
            ),
            BlockingError::Worker(_) => Self::internal_error(path, error),
        }
    }
}
