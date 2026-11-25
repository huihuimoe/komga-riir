//! Kotlin `FileSystemScanner.scanRootFolder`: walks the library root like Java `walkFileTree` with
//! `FOLLOW_LINKS`, groups book files into series by directory, and finds their sidecars.

use std::fs::{self, Metadata};
use std::path::{Path, PathBuf};

use jiff::civil::DateTime;
use jiff::tz::TimeZone;
use komga_types::{Book, BookId, FileUrl, Library, LibraryId, Series, SeriesId, Sidecar};
use thiserror::Error;
use tracing::{info, warn};

use super::ignore_case;
use super::sidecar_names::{self, BookSidecarCandidate};
use crate::support::file_time;

#[derive(Debug, Error)]
pub enum ScanError {
    /// Kotlin `DirectoryNotFoundException` (`ERR_1016`). A root that cannot be listed also
    /// counts, so that an unreadable root never reads as an empty library.
    #[error("Folder is not accessible: {}", .0.display())]
    DirectoryNotFound(PathBuf),
    /// A root without books while the library still has series, like an unmounted network
    /// share's mount point: no scan deletes its whole content.
    #[error("Folder has no books, but the library has series: {}", .0.display())]
    Emptied(PathBuf),
}

#[derive(Default)]
pub(crate) struct ScanResult {
    /// Scanned series with their books, in Kotlin's post-order: a directory after its
    /// subdirectories.
    pub(crate) series: Vec<(Series, Vec<Book>)>,
    /// Sidecars of directories with books, in the same order; series sidecars of a directory
    /// precede its book sidecars.
    pub(super) sidecars: Vec<Sidecar>,
    pub(super) unread: Unread,
}

/// Directories the walk could not fully list. Kotlin carries on as if they were empty; keeps
/// what is stored under them, since their absence from the scan proves nothing.
#[derive(Default)]
pub(super) struct Unread(Vec<FileUrl>);

impl Unread {
    /// Whether `url` is a directory the walk could not fully list or lies under one.
    pub(super) fn contains(&self, url: &FileUrl) -> bool {
        self.0
            .iter()
            .any(|directory| url.as_str().starts_with(directory.as_str()))
    }

    fn push(&mut self, directory: &Path) {
        self.0.push(FileUrl::directory(directory));
    }
}

struct Walk<'a> {
    library: &'a Library,
    extensions: Vec<&'static str>,
    time_zone: TimeZone,
    /// Canonical paths of the directories being visited, to detect symbolic link loops.
    ancestors: Vec<PathBuf>,
    result: ScanResult,
}

/// The files of one directory that the post-visit turns into series, books, and sidecars.
#[derive(Default)]
struct DirectoryFiles {
    books: Vec<Book>,
    series_sidecars: Vec<Sidecar>,
    /// Book sidecars are only matched once the directory's books are known.
    book_sidecar_candidates: Vec<BookSidecarCandidate>,
}

pub(crate) fn scan_root_folder(library: &Library) -> Result<ScanResult, ScanError> {
    let root = &library.root;
    let mut extensions = Vec::new();
    if library.scan_cbx {
        extensions.extend(["cbz", "zip", "cbr", "rar"]);
    }
    if library.scan_pdf {
        extensions.push("pdf");
    }
    if library.scan_epub {
        extensions.extend(["epub", "mobi"]);
    }
    info!(root = %root.display(), ?extensions, exclusions = ?library.scan_directory_exclusions,
        force_directory_modified_time = library.scan_force_modified_time, "Scanning folder");
    let not_found = || ScanError::DirectoryNotFound(root.clone());
    let metadata = fs::metadata(root).map_err(|_| not_found())?;
    if !metadata.is_dir() || fs::read_dir(root).is_err() {
        return Err(not_found());
    }
    let mut walk = Walk {
        library,
        extensions,
        time_zone: TimeZone::system(),
        ancestors: Vec::new(),
        result: ScanResult::default(),
    };
    walk.directory(root, &metadata);
    let result = walk.result;
    let books: usize = result.series.iter().map(|(_, books)| books.len()).sum();
    info!(
        series = result.series.len(),
        books,
        sidecars = result.sidecars.len(),
        "Scanned folder"
    );
    Ok(result)
}

impl Walk<'_> {
    fn directory(&mut self, directory: &Path, metadata: &Metadata) {
        let name = file_name(directory);
        let path = directory.to_string_lossy();
        let excluded = self
            .library
            .scan_directory_exclusions
            .iter()
            .any(|exclusion| ignore_case::contains(&path, exclusion));
        let skipped = name.starts_with('.') || excluded;
        let entries = match fs::read_dir(directory) {
            Ok(entries) => entries,
            Err(error) => {
                warn!(path = %directory.display(), %error, "Could not access");
                if !skipped {
                    self.result.unread.push(directory);
                }
                return;
            }
        };
        if skipped {
            return;
        }
        let series_modified = file_time::updated_time(metadata, &self.time_zone);
        let canonical = match fs::canonicalize(directory) {
            Ok(canonical) => canonical,
            Err(error) => {
                warn!(path = %directory.display(), %error, "Could not access");
                self.result.unread.push(directory);
                return;
            }
        };
        if self.ancestors.contains(&canonical) {
            warn!(path = %directory.display(), "Could not access: file system loop");
            return;
        }
        self.ancestors.push(canonical);
        let mut files = DirectoryFiles::default();
        for entry in entries {
            let entry = match entry {
                Ok(entry) => entry.path(),
                Err(error) => {
                    // Java ends the iteration and still post-visits the directory.
                    warn!(path = %directory.display(), %error, "Could not list");
                    self.result.unread.push(directory);
                    break;
                }
            };
            self.entry(directory, &entry, &mut files);
        }
        self.ancestors.pop();
        if !files.books.is_empty() {
            self.post_visit(directory, &path, name, series_modified, files);
        }
    }

    fn entry(&mut self, directory: &Path, path: &Path, files: &mut DirectoryFiles) {
        let metadata = match fs::metadata(path) {
            Ok(metadata) => metadata,
            // A broken link: Java visits the link itself, which is never a book.
            Err(_) if fs::symlink_metadata(path).is_ok_and(|link| link.is_symlink()) => return,
            // The entry may be a book or a subdirectory: the whole directory is undecided.
            Err(error) => {
                warn!(path = %path.display(), %error, "Could not access");
                self.result.unread.push(directory);
                return;
            }
        };
        if metadata.is_dir() {
            self.directory(path, &metadata);
            return;
        }
        let name = file_name(path);
        let updated_time = file_time::updated_time(&metadata, &self.time_zone);
        let (stem, extension) = name.rsplit_once('.').unwrap_or((&name, ""));
        if !name.starts_with('.') && self.extensions.contains(&extension.to_lowercase().as_str()) {
            let book = book(path, stem, &metadata, updated_time, &self.library.id);
            files.books.push(book);
        }
        if let Some(kind) = sidecar_names::series_sidecar(&name) {
            files.series_sidecars.push(Sidecar {
                url: FileUrl::file(path),
                parent_url: FileUrl::directory(directory),
                last_modified_time: updated_time,
                kind,
            });
        }
        if sidecar_names::is_book_sidecar_candidate(&name) {
            files.book_sidecar_candidates.push(BookSidecarCandidate {
                file_name: name,
                url: FileUrl::file(path),
                last_modified_time: updated_time,
            });
        }
    }

    fn post_visit(
        &mut self,
        directory: &Path,
        path: &str,
        name: String,
        modified: DateTime,
        files: DirectoryFiles,
    ) {
        let DirectoryFiles {
            books,
            series_sidecars,
            book_sidecar_candidates,
        } = files;
        let book_sidecars = sidecar_names::match_book_sidecars(&books, book_sidecar_candidates);
        let oneshots = self
            .library
            .oneshots_directory
            .as_deref()
            .filter(|oneshots| !oneshots.chars().all(is_kotlin_whitespace));
        if oneshots.is_some_and(|oneshots| ignore_case::contains(path, oneshots)) {
            for book in books {
                let series = Series {
                    id: SeriesId::generate(),
                    name: book.name.clone(),
                    url: book.url.clone(),
                    file_last_modified: book.file_last_modified,
                    library_id: self.library.id.clone(),
                    book_count: 0,
                    deleted_date: None,
                    oneshot: true,
                };
                self.result.series.push((
                    series,
                    vec![Book {
                        oneshot: true,
                        ..book
                    }],
                ));
            }
            self.result.sidecars.extend(book_sidecars);
            return;
        }
        let file_last_modified = if self.library.scan_force_modified_time {
            books
                .iter()
                .map(|book| book.file_last_modified)
                .fold(modified, DateTime::max)
        } else {
            modified
        };
        let series = Series {
            id: SeriesId::generate(),
            name: if name.chars().all(is_kotlin_whitespace) {
                directory.to_string_lossy().into_owned()
            } else {
                name
            },
            url: FileUrl::directory(directory),
            file_last_modified,
            library_id: self.library.id.clone(),
            book_count: 0,
            deleted_date: None,
            oneshot: false,
        };
        self.result.series.push((series, books));
        // Oneshot directories have no series sidecars.
        self.result.sidecars.extend(series_sidecars);
        self.result.sidecars.extend(book_sidecars);
    }
}

/// Kotlin `FileSystemScanner.scanFile`: the book at `path` whatever its extension, `None` when
/// it cannot be read.
pub(crate) fn scan_file(path: &Path, library_id: &LibraryId) -> Option<Book> {
    let metadata = fs::metadata(path).ok().filter(Metadata::is_file)?;
    let name = file_name(path);
    let stem = name
        .rsplit_once('.')
        .map_or(name.as_str(), |(stem, _)| stem);
    let updated_time = file_time::updated_time(&metadata, &TimeZone::system());
    Some(book(path, stem, &metadata, updated_time, library_id))
}

fn book(
    path: &Path,
    name: &str,
    metadata: &Metadata,
    file_last_modified: DateTime,
    library_id: &LibraryId,
) -> Book {
    Book {
        id: BookId::generate(),
        name: name.to_owned(),
        url: FileUrl::file(path),
        file_last_modified,
        file_size: i64::try_from(metadata.len()).unwrap_or(i64::MAX),
        file_hash: String::new(),
        file_hash_koreader: String::new(),
        number: 0,
        series_id: SeriesId::from(String::new()),
        library_id: library_id.clone(),
        deleted_date: None,
        oneshot: false,
    }
}

fn file_name(path: &Path) -> String {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default()
}

/// Kotlin `Char.isWhitespace()`, for `isBlank`.
fn is_kotlin_whitespace(character: char) -> bool {
    (character.is_whitespace() && character != '\u{85}')
        || ('\u{1c}'..='\u{1f}').contains(&character)
}

#[cfg(all(test, unix))]
mod tests;
