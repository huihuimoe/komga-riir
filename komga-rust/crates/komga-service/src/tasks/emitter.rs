//! Kotlin `TaskEmitter` operations that select books before queuing tasks for them.

use komga_db::{BookDao, BookHash, DbError, DbTransaction};
use komga_scheduler::{LOW_PRIORITY, LOWEST_PRIORITY, Task};
use komga_types::Library;

/// Kotlin `BookConverter.mediaTypeToExtension`, extended with `.mobi` (spec §16.4).
pub(crate) const MEDIA_TYPE_EXTENSIONS: [(&str, &str); 6] = [
    ("application/x-rar-compressed; version=4", "cbr"),
    ("application/x-rar-compressed; version=5", "cbr"),
    ("application/zip", "cbz"),
    ("application/pdf", "pdf"),
    ("application/epub+zip", "epub"),
    ("application/x-mobipocket-ebook", "mobi"),
];

/// The follow-ups Kotlin `TaskHandler` queues after a successful `ScanLibrary`, in its order.
pub(crate) async fn scan_follow_ups(
    tx: &mut DbTransaction,
    library: &Library,
) -> Result<Vec<Task>, DbError> {
    let mut tasks = analyze_unknown_and_outdated_books(tx, library).await?;
    tasks.extend(repair_extensions(tx, library, LOW_PRIORITY).await?);
    tasks.extend([
        Task::find_books_to_convert(library.id.clone()).lowest(),
        Task::find_books_with_missing_page_hash(library.id.clone()).lowest(),
        Task::find_duplicate_pages_to_delete(library.id.clone()).lowest(),
    ]);
    tasks.extend(hash_books_without_hash(tx, library, BookHash::File).await?);
    tasks.extend(hash_books_without_hash(tx, library, BookHash::Koreader).await?);
    Ok(tasks)
}

/// The tasks Kotlin `LibraryLifecycle.updateLibrary` queues for options switched on.
pub(crate) async fn enabled_option_follow_ups(
    tx: &mut DbTransaction,
    current: &Library,
    changed: &Library,
) -> Result<Vec<Task>, DbError> {
    let enabled = |option: fn(&Library) -> bool| option(changed) && !option(current);
    let mut tasks = Vec::new();
    if enabled(|library| library.hash_files) {
        tasks.extend(hash_books_without_hash(tx, changed, BookHash::File).await?);
    }
    if enabled(|library| library.hash_koreader) {
        tasks.extend(hash_books_without_hash(tx, changed, BookHash::Koreader).await?);
    }
    if enabled(|library| library.hash_pages) {
        tasks.push(Task::find_books_with_missing_page_hash(changed.id.clone()).lowest());
    }
    if enabled(|library| library.repair_extensions) {
        tasks.extend(repair_extensions(tx, changed, LOWEST_PRIORITY).await?);
    }
    if enabled(|library| library.convert_to_cbz) {
        tasks.push(Task::find_books_to_convert(changed.id.clone()).lowest());
    }
    Ok(tasks)
}

async fn analyze_unknown_and_outdated_books(
    tx: &mut DbTransaction,
    library: &Library,
) -> Result<Vec<Task>, DbError> {
    let books =
        BookDao::find_all_by_library_and_media_statuses(tx, &library.id, &["UNKNOWN", "OUTDATED"])
            .await?;
    Ok(books
        .into_iter()
        .map(|book| Task::analyze_book(book.id, book.series_id))
        .collect())
}

async fn repair_extensions(
    tx: &mut DbTransaction,
    library: &Library,
    priority: i32,
) -> Result<Vec<Task>, DbError> {
    let mut tasks = Vec::new();
    if !library.repair_extensions {
        return Ok(tasks);
    }
    for (media_type, extension) in MEDIA_TYPE_EXTENSIONS {
        let books = BookDao::find_all_by_library_and_mismatched_extension(
            tx,
            &library.id,
            media_type,
            extension,
        )
        .await?;
        tasks.extend(
            books.into_iter().map(|book| {
                Task::repair_extension(book.id, book.series_id).with_priority(priority)
            }),
        );
    }
    Ok(tasks)
}

/// `hashBooksWithoutHash` and `hashBooksWithoutHashKoreader`, when the library enables them.
async fn hash_books_without_hash(
    tx: &mut DbTransaction,
    library: &Library,
    hash: BookHash,
) -> Result<Vec<Task>, DbError> {
    let enabled = match hash {
        BookHash::File => library.hash_files,
        BookHash::Koreader => library.hash_koreader,
    };
    if !enabled {
        return Ok(Vec::new());
    }
    let books = BookDao::find_all_by_library_with_empty_hash(tx, &library.id, hash).await?;
    Ok(books
        .into_iter()
        .map(|book| match hash {
            BookHash::File => Task::hash_book(book.id).lowest(),
            BookHash::Koreader => Task::hash_book_koreader(book.id).lowest(),
        })
        .collect())
}
