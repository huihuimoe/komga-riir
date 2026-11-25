//! One generation of the index: a Tantivy index written with fixed n-gram settings.

use std::fs;
use std::path::Path;
use std::sync::Mutex;
use std::vec;

use komga_types::SearchEntity;
use tantivy::collector::{Count, TopDocs};
use tantivy::query::{BooleanQuery, Occur, TermQuery};
use tantivy::schema::{Field, IndexRecordOption, Value};
use tantivy::tokenizer::PreTokenizedString;
use tantivy::{
    DocAddress, Index, IndexReader, IndexSettings, IndexWriter, ReloadPolicy, Searcher,
    TantivyDocument, TantivyError, Term,
};

use super::SearchError;
use crate::analyzer::{self, NgramOptions};
use crate::document::{self, IndexUpdate, SearchDocument};
use crate::query;
use crate::schema::SearchSchema;

/// Memory the single indexing thread buffers before it flushes a segment.
const WRITER_MEMORY_BYTES: usize = 50_000_000;

/// Documents indexed with the n-gram settings in force when the generation was created, which its
/// later updates keep using.
pub struct IndexGeneration {
    pub(super) number: u64,
    pub(super) ngram: NgramOptions,
    index: Index,
    schema: SearchSchema,
    reader: IndexReader,
    /// Created lazily on the first write so read-only opens and empty rebuilds avoid allocating
    /// [`WRITER_MEMORY_BYTES`] and spawning indexing threads.
    writer: Mutex<Option<IndexWriter>>,
}

impl IndexGeneration {
    /// Creates an empty generation in `path`, replacing whatever is there.
    pub(super) fn create(
        path: &Path,
        number: u64,
        ngram: NgramOptions,
    ) -> Result<Self, SearchError> {
        if path.exists() {
            fs::remove_dir_all(path)?;
        }
        fs::create_dir_all(path)?;
        let schema = SearchSchema::new();
        let index = Index::create(
            open_directory(path)?,
            schema.schema.clone(),
            IndexSettings::default(),
        )?;
        Self::new(index, schema, number, ngram)
    }

    /// Opens the generation in `path`, which must hold an index of the current schema.
    pub(super) fn open(path: &Path, number: u64, ngram: NgramOptions) -> Result<Self, SearchError> {
        let schema = SearchSchema::new();
        let index = Index::open(open_directory(path)?)?;
        if index.schema() != schema.schema {
            return Err(
                TantivyError::SchemaError("the index has another schema".to_owned()).into(),
            );
        }
        Self::new(index, schema, number, ngram)
    }

    fn new(
        index: Index,
        schema: SearchSchema,
        number: u64,
        ngram: NgramOptions,
    ) -> Result<Self, SearchError> {
        let reader = index
            .reader_builder()
            .reload_policy(ReloadPolicy::Manual)
            .try_into()?;
        Ok(Self {
            number,
            ngram,
            index,
            schema,
            reader,
            writer: Mutex::new(None),
        })
    }

    /// Applies the updates in order and commits them; searches see them once this returns.
    ///
    /// # Errors
    /// Fails when the index cannot be written; then none of the updates are visible.
    pub fn apply(&self, updates: &[IndexUpdate]) -> Result<(), SearchError> {
        let mut guard = self
            .writer
            .lock()
            .expect("a panic while staging would leave stray changes for the next commit");
        let writer = match guard.as_mut() {
            Some(writer) => writer,
            None => guard.insert(self.index.writer_with_num_threads(1, WRITER_MEMORY_BYTES)?),
        };
        let applied = updates
            .iter()
            .try_for_each(|update| self.stage(writer, update))
            .and_then(|()| writer.commit().map(|_| ()));
        if let Err(error) = applied {
            writer.rollback()?;
            return Err(error.into());
        }
        self.reader.reload()?;
        Ok(())
    }

    fn stage(&self, writer: &IndexWriter, update: &IndexUpdate) -> tantivy::Result<()> {
        match update {
            IndexUpdate::Upsert(document) => {
                writer.delete_term(self.key(document.entity, &document.id));
                writer.add_document(self.document(document))?;
            }
            IndexUpdate::Delete { entity, id } => {
                writer.delete_term(self.key(*entity, id));
            }
        }
        Ok(())
    }

    fn key(&self, entity: SearchEntity, id: &str) -> Term {
        Term::from_field_text(self.schema.key, &key_text(entity, id))
    }

    fn document(&self, source: &SearchDocument) -> TantivyDocument {
        let mut document = TantivyDocument::new();
        document.add_text(self.schema.key, key_text(source.entity, &source.id));
        document.add_text(self.schema.entity_type, source.entity.type_name());
        document.add_text(self.schema.id, &source.id);
        let mut analyzer = analyzer::index_analyzer(self.ngram);
        for (name, value) in &source.fields {
            let target = self.schema.target(name);
            let mut stream = analyzer.token_stream(value);
            let mut tokens = Vec::new();
            while let Some(token) = stream.next() {
                let mut token = token.clone();
                token.text = target.term_text(&token.text);
                tokens.push(token);
            }
            let text = PreTokenizedString {
                text: value.clone(),
                tokens,
            };
            document.add_pre_tokenized_text(target.field, text);
        }
        document
    }

    pub(super) fn hits(
        &self,
        entity: SearchEntity,
        query: &str,
    ) -> Result<SearchHits, SearchError> {
        let searcher = self.reader.searcher();
        let mut hits = SearchHits {
            searcher,
            id: self.schema.id,
            addresses: Vec::new().into_iter(),
        };
        let Ok(parsed) = query::build(query, document::default_fields(entity), &self.schema) else {
            return Ok(hits);
        };
        let of_entity = TermQuery::new(
            Term::from_field_text(self.schema.entity_type, entity.type_name()),
            IndexRecordOption::Basic,
        );
        let query = BooleanQuery::new(vec![
            (Occur::Must, parsed),
            (Occur::Must, Box::new(of_entity)),
        ]);
        let count = hits.searcher.search(&query, &Count)?;
        if count == 0 {
            return Ok(hits);
        }
        let ranked = hits
            .searcher
            .search(&query, &TopDocs::with_limit(count).order_by_score())?;
        hits.addresses = ranked
            .into_iter()
            .map(|(_, address)| address)
            .collect::<Vec<DocAddress>>()
            .into_iter();
        Ok(hits)
    }
}

/// The entities matching a query, most relevant first, as one fixed reader of the generation saw
/// them; their IDs are read a batch at a time.
pub struct SearchHits {
    searcher: Searcher,
    id: Field,
    addresses: vec::IntoIter<DocAddress>,
}

impl SearchHits {
    /// How many hits are left to read.
    #[must_use]
    pub fn len(&self) -> usize {
        self.addresses.len()
    }

    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.addresses.len() == 0
    }

    /// The IDs of the next `size` hits at most; empty once all are read. Blocks on disk I/O.
    ///
    /// # Errors
    /// Fails when the stored IDs cannot be read.
    pub fn next_batch(&mut self, size: usize) -> Result<Vec<String>, SearchError> {
        self.addresses
            .by_ref()
            .take(size)
            .map(|address| {
                let document: TantivyDocument = self.searcher.doc(address)?;
                let id = document
                    .get_first(self.id)
                    .and_then(|value| value.as_str())
                    .expect("every document stores its ID");
                Ok(id.to_owned())
            })
            .collect()
    }
}

fn key_text(entity: SearchEntity, id: &str) -> String {
    format!("{}:{id}", entity.type_name())
}

#[cfg(not(windows))]
fn open_directory(
    path: &Path,
) -> Result<tantivy::directory::MmapDirectory, tantivy::directory::error::OpenDirectoryError> {
    tantivy::directory::MmapDirectory::open(path)
}

#[cfg(not(windows))]
pub(super) fn retry_permission_denied<T, E>(
    mut op: impl FnMut() -> Result<T, E>,
    _is_denied: impl Fn(&E) -> bool,
) -> Result<T, E> {
    op()
}

#[cfg(windows)]
use windows::open_directory;
#[cfg(windows)]
pub(super) use windows::retry_permission_denied;

#[cfg(windows)]
mod windows {
    use std::io;
    use std::path::Path;
    use std::sync::Arc;
    use std::time::Duration;

    use tantivy::directory::MmapDirectory;
    use tantivy::directory::error::{
        DeleteError, LockError, OpenDirectoryError, OpenReadError, OpenWriteError,
    };
    use tantivy::directory::{
        Directory, DirectoryLock, FileHandle, Lock, WatchCallback, WatchHandle, WritePtr,
    };

    pub(in super::super) fn open_directory(
        path: &Path,
    ) -> Result<WindowsDirectory, OpenDirectoryError> {
        MmapDirectory::open(path).map(WindowsDirectory)
    }

    /// Retries an operation on Windows `ERROR_ACCESS_DENIED` caused by transient filesystem filter
    /// locks (e.g. antivirus or search indexer scans right after creating or replacing index files).
    pub(in super::super) fn retry_permission_denied<T, E>(
        mut op: impl FnMut() -> Result<T, E>,
        is_denied: impl Fn(&E) -> bool,
    ) -> Result<T, E> {
        let mut delay = Duration::from_millis(10);
        for _ in 0..5 {
            match op() {
                Err(error) if is_denied(&error) => {
                    std::thread::sleep(delay);
                    delay *= 2;
                }
                result => return result,
            }
        }
        op()
    }

    /// `MmapDirectory` wrapper that retries transient Windows sharing/permission errors when
    /// Tantivy creates, deletes, or atomically replaces `.managed.json` and segment files.
    #[derive(Clone, Debug)]
    pub(in super::super) struct WindowsDirectory(MmapDirectory);

    impl Directory for WindowsDirectory {
        fn get_file_handle(&self, path: &Path) -> Result<Arc<dyn FileHandle>, OpenReadError> {
            self.0.get_file_handle(path)
        }

        fn delete(&self, path: &Path) -> Result<(), DeleteError> {
            retry_permission_denied(
                || self.0.delete(path),
                |error| {
                    matches!(
                        error,
                        DeleteError::IoError { io_error, .. }
                            if io_error.kind() == io::ErrorKind::PermissionDenied
                    )
                },
            )
        }

        fn exists(&self, path: &Path) -> Result<bool, OpenReadError> {
            self.0.exists(path)
        }

        fn open_write(&self, path: &Path) -> Result<WritePtr, OpenWriteError> {
            retry_permission_denied(
                || self.0.open_write(path),
                |error| {
                    matches!(
                        error,
                        OpenWriteError::IoError { io_error, .. }
                            if io_error.kind() == io::ErrorKind::PermissionDenied
                    )
                },
            )
        }

        fn atomic_read(&self, path: &Path) -> Result<Vec<u8>, OpenReadError> {
            self.0.atomic_read(path)
        }

        fn atomic_write(&self, path: &Path, data: &[u8]) -> io::Result<()> {
            retry_permission_denied(
                || self.0.atomic_write(path, data),
                |error| error.kind() == io::ErrorKind::PermissionDenied,
            )
        }

        fn sync_directory(&self) -> io::Result<()> {
            self.0.sync_directory()
        }

        fn acquire_lock(&self, lock: &Lock) -> Result<DirectoryLock, LockError> {
            self.0.acquire_lock(lock)
        }

        fn watch(&self, watch_callback: WatchCallback) -> tantivy::Result<WatchHandle> {
            self.0.watch(watch_callback)
        }
    }
}
