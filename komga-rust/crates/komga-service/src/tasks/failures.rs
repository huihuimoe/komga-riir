//! How each service error fails a task: retried later or failed for good.

use komga_scheduler::TaskFailure;

use crate::artwork::ArtworkError;
use crate::books::analysis::BookAnalysisError;
use crate::books::conversion::ConversionError;
use crate::books::hashing::HashingError;
use crate::books::importing::ImportError;
use crate::catalog::search_index::SearchIndexError;
use crate::library::file_deletion::FileDeletionError;
use crate::library::file_operation::FileOperationError;
use crate::library::scanning::LibraryContentError;
use crate::library::series::SeriesError;
use crate::metadata::MetadataError;
use crate::settings::SettingsError;

pub(super) fn deletion_failure(error: FileDeletionError) -> TaskFailure {
    match error {
        FileDeletionError::Db(_) | FileDeletionError::FileOperation(FileOperationError::Db(_)) => {
            TaskFailure::Retryable(error.to_string())
        }
        FileDeletionError::FileOperation(_)
        | FileDeletionError::Io(_)
        | FileDeletionError::Worker(_) => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn conversion_failure(error: ConversionError) -> TaskFailure {
    match error {
        ConversionError::Db(_) | ConversionError::FileOperation(FileOperationError::Db(_)) => {
            TaskFailure::Retryable(error.to_string())
        }
        ConversionError::FileNotFound(_)
        | ConversionError::MediaUnsupported(..)
        | ConversionError::NotRepairable(_)
        | ConversionError::MediaNotReady
        | ConversionError::Unverified(_)
        | ConversionError::FileOperation(_)
        | ConversionError::Rar(_)
        | ConversionError::Zip(_)
        | ConversionError::Io(_)
        | ConversionError::Worker(_) => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn failure(error: LibraryContentError) -> TaskFailure {
    match error {
        // Database failures (busy, I/O) and unreadable files (offline NAS) are usually temporary.
        LibraryContentError::Db(_)
        | LibraryContentError::Hash(_)
        | LibraryContentError::Series(SeriesError::Db(_))
        | LibraryContentError::Settings(SettingsError::Db(_)) => {
            TaskFailure::Retryable(error.to_string())
        }
        LibraryContentError::Scan(_)
        | LibraryContentError::Worker(_)
        | LibraryContentError::Series(SeriesError::MissingBookMetadata(_))
        | LibraryContentError::Settings(_) => TaskFailure::Permanent(error.to_string()),
        LibraryContentError::Interrupted => TaskFailure::Interrupted,
    }
}

pub(super) fn analysis_failure(error: BookAnalysisError) -> TaskFailure {
    match error {
        BookAnalysisError::Db(_) | BookAnalysisError::Io(_) => {
            TaskFailure::Retryable(error.to_string())
        }
        BookAnalysisError::NotFound
        | BookAnalysisError::Forbidden
        | BookAnalysisError::Worker(_)
        | BookAnalysisError::InvalidUrl(_)
        | BookAnalysisError::MissingMedia(_)
        | BookAnalysisError::MissingCodec(_) => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn artwork_failure(error: ArtworkError) -> TaskFailure {
    match error {
        // Database failures and unreadable artwork (busy database, offline NAS) are usually
        // temporary.
        ArtworkError::Db(_)
        | ArtworkError::Settings(SettingsError::Db(_))
        | ArtworkError::Io(_)
        | ArtworkError::Blocking(_) => TaskFailure::Retryable(error.to_string()),
        ArtworkError::NotFound
        | ArtworkError::Restricted
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
        | ArtworkError::OtherOwner
        | ArtworkError::NotUploaded
        | ArtworkError::NotImage
        | ArtworkError::Oneshot => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn hashing_failure(error: HashingError) -> TaskFailure {
    match error {
        // A busy database or an offline NAS is usually temporary.
        HashingError::Db(_) | HashingError::Io(_) => TaskFailure::Retryable(error.to_string()),
        HashingError::Worker(_)
        | HashingError::InvalidUrl(_)
        | HashingError::Archive(_)
        | HashingError::Image(_) => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn metadata_failure(error: MetadataError) -> TaskFailure {
    match error {
        MetadataError::Db(_) => TaskFailure::Retryable(error.to_string()),
        MetadataError::MissingMedia(_) => TaskFailure::Permanent(error.to_string()),
        MetadataError::NotFound | MetadataError::Forbidden => {
            unreachable!("Only HTTP requests are refused or ask for a missing object: {error}")
        }
    }
}

pub(super) fn import_failure(error: ImportError) -> TaskFailure {
    match error {
        // A busy database is usually temporary; files are left as they were.
        ImportError::Db(_)
        | ImportError::Series(SeriesError::Db(_))
        | ImportError::FileOperation(FileOperationError::Db(_)) => {
            TaskFailure::Retryable(error.to_string())
        }
        ImportError::SourceNotFound
        | ImportError::Oneshot
        | ImportError::InLibrary
        | ImportError::NotInSeries
        | ImportError::TargetExists
        | ImportError::NotScanned
        | ImportError::FileOperation(_)
        | ImportError::Series(SeriesError::MissingBookMetadata(_))
        | ImportError::Io(_)
        | ImportError::Worker(_) => TaskFailure::Permanent(error.to_string()),
    }
}

pub(super) fn index_result(result: Result<(), SearchIndexError>) -> Result<(), TaskFailure> {
    result.map_err(|error| match error {
        // A busy database or a full disk is usually temporary.
        SearchIndexError::Db(_) | SearchIndexError::Index(_) => {
            TaskFailure::Retryable(error.to_string())
        }
        SearchIndexError::Panicked(_) => TaskFailure::Permanent(error.to_string()),
        SearchIndexError::Interrupted => TaskFailure::Interrupted,
        SearchIndexError::OverBudget(_) => {
            unreachable!("Only list searches have a budget: {error}")
        }
    })
}
