//! How analysis failures are stored: Kotlin's `analyze` catch clauses, except for OS read
//! failures the file may not repeat.

use std::error::Error;
use std::io;
use std::path::Path;

use komga_archive::epub::EpubError;
use komga_archive::mobi::MobiError;
use komga_archive::pdf::PdfError;
use komga_archive::rar::RarError;
use komga_archive::zip::ZipError;
use komga_types::{Media, MediaStatus};
use tracing::error;

use super::BookAnalysisError;

/// Kotlin `analyze` catch clauses: `AccessDeniedException`, `NoSuchFileException`, then any.
pub(super) fn io_comment(cause: &io::Error) -> String {
    match cause.kind() {
        io::ErrorKind::PermissionDenied => "ERR_1000",
        io::ErrorKind::NotFound => "ERR_1018",
        _ => "ERR_1005",
    }
    .to_owned()
}

/// The error media Kotlin stores for `cause`, unless the read failed in a way the file may not
/// fail next time.
pub(super) fn failed(
    path: &Path,
    cause: &(dyn Error + 'static),
    comment: String,
) -> Result<Media, BookAnalysisError> {
    if let Some(error) = transient(cause) {
        return Err(error);
    }
    error!(path = %path.display(), %cause, "Error while analyzing book");
    Ok(Media::failed(MediaStatus::Error, Some(comment)))
}

/// An OS read failure other than a missing or forbidden file, such as a dropped network share or
/// an unplugged disk. Kotlin stores it like a damaged file; here it fails the task, which keeps
/// the stored media and retries. Archive and image decoders report damage without
/// an OS error code.
pub(super) fn transient(cause: &(dyn Error + 'static)) -> Option<BookAnalysisError> {
    let mut next = Some(cause);
    while let Some(cause) = next {
        if let Some(io) = io_error(cause) {
            return match (io.kind(), io.raw_os_error()) {
                (io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied, _) | (_, None) => None,
                (_, Some(code)) => Some(BookAnalysisError::Io(io::Error::from_raw_os_error(code))),
            };
        }
        next = cause.source();
    }
    None
}

/// The `io::Error` that `cause` is; `#[error(transparent)]` variants are missing from the
/// `source` chain.
fn io_error<'a>(cause: &'a (dyn Error + 'static)) -> Option<&'a io::Error> {
    let zip = |cause: &'a ZipError| match cause {
        ZipError::Io(io) => Some(io),
        _ => None,
    };
    if let Some(io) = cause.downcast_ref::<io::Error>() {
        Some(io)
    } else if let Some(cause) = cause.downcast_ref::<ZipError>() {
        zip(cause)
    } else if let Some(EpubError::Zip(cause)) = cause.downcast_ref() {
        zip(cause)
    } else if let Some(MobiError::Io(io)) = cause.downcast_ref() {
        Some(io)
    } else if let Some(PdfError::Io(io)) = cause.downcast_ref() {
        Some(io)
    } else if let Some(RarError::Io(io)) = cause.downcast_ref() {
        Some(io)
    } else {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// `EIO`, `ENOENT` and `EACCES`, which share their numbers on Linux and macOS.
    #[cfg(unix)]
    const EIO: i32 = 5;
    #[cfg(unix)]
    const ENOENT: i32 = 2;
    #[cfg(unix)]
    const EACCES: i32 = 13;
    /// `ERROR_READ_FAULT`, `ERROR_FILE_NOT_FOUND` and `ERROR_ACCESS_DENIED`.
    #[cfg(windows)]
    const EIO: i32 = 30;
    #[cfg(windows)]
    const ENOENT: i32 = 2;
    #[cfg(windows)]
    const EACCES: i32 = 5;

    fn os(code: i32) -> io::Error {
        io::Error::from_raw_os_error(code)
    }

    #[test]
    fn only_os_read_failures_of_an_existing_file_are_transient() {
        let transient_causes: [&(dyn Error + 'static); 6] = [
            &os(EIO),
            &ZipError::Io(os(EIO)),
            &EpubError::Zip(ZipError::Io(os(EIO))),
            &MobiError::Io(os(EIO)),
            &PdfError::Io(os(EIO)),
            &RarError::Io(os(EIO)),
        ];
        for cause in transient_causes {
            assert!(
                matches!(transient(cause), Some(BookAnalysisError::Io(io)) if io.raw_os_error() == Some(EIO)),
                "{cause:?}"
            );
        }

        let damaged = io::Error::new(io::ErrorKind::InvalidData, "broken archive");
        let stored_causes: [&(dyn Error + 'static); 4] = [
            &ZipError::Io(damaged),
            &ZipError::Io(os(ENOENT)),
            &PdfError::Io(os(EACCES)),
            &ZipError::LimitExceeded("too many entries"),
        ];
        for cause in stored_causes {
            assert!(transient(cause).is_none(), "{cause:?}");
        }
    }
}
