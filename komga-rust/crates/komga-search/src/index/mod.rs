//! The on-disk index: the current generation, the manifest naming it, and switching to a rebuilt
//! one.
//!
//! `<data-directory>/tantivy` holds one directory per generation and `generation.json`, which
//! names the current one; a generation becomes current only once the manifest is replaced. Every
//! other entry there is left over from a rebuild that did not finish, or from an older layout,
//! and is removed.

mod generation;
#[cfg(test)]
mod tests;

use std::fmt;
use std::fs::{self, File};
use std::io::{self, Write};
use std::path::{Path, PathBuf};
use std::sync::{Arc, RwLock};

pub use generation::{IndexGeneration, SearchHits};
use komga_types::SearchEntity;
use serde::{Deserialize, Serialize};

use crate::analyzer::NgramOptions;

/// The index directory inside `komga.lucene.data-directory`, which may still hold Komga's
/// Lucene index.
const INDEX_DIRECTORY: &str = "tantivy";
const MANIFEST: &str = "generation.json";
const MANIFEST_TEMPORARY: &str = "generation.json.tmp";
/// Changes whenever the schema, the documents, or their analysis change, so that generations
/// written before are rebuilt.
const FORMAT: u32 = 1;

/// The full-text index of books, series, collections, and read lists. Every method blocks on
/// disk I/O.
pub struct SearchIndex {
    root: PathBuf,
    ngram: NgramOptions,
    current: RwLock<Current>,
}

enum Current {
    Open(Arc<IndexGeneration>),
    Unusable(RebuildReason),
}

/// Why the index must be rebuilt.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RebuildReason {
    Missing,
    OtherFormat,
    Unreadable(String),
    NgramChanged,
}

impl fmt::Display for RebuildReason {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Missing => f.write_str("there is no search index"),
            Self::OtherFormat => f.write_str("the search index has another format"),
            Self::Unreadable(error) => write!(f, "the search index cannot be read: {error}"),
            Self::NgramChanged => f.write_str("the index analyzer n-gram settings changed"),
        }
    }
}

#[derive(Debug, thiserror::Error)]
pub enum SearchError {
    #[error("search index I/O failed: {0}")]
    Io(#[from] io::Error),
    #[error("search index failed: {0}")]
    Index(#[from] tantivy::TantivyError),
    #[error("search index directory is unusable: {0}")]
    Directory(#[from] tantivy::directory::error::OpenDirectoryError),
    #[error("the search index is unavailable until it is rebuilt: {0}")]
    Unavailable(RebuildReason),
}

#[derive(Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
struct Manifest {
    format: u32,
    generation: u64,
    min_gram: usize,
    max_gram: usize,
    preserve_original: bool,
}

impl SearchIndex {
    /// Opens the current generation under `data_directory`, if it is usable, and removes every
    /// other one. `ngram` applies to the generations created from now on.
    ///
    /// # Errors
    /// Fails when the directory cannot be created, listed, or cleaned up.
    pub fn open(data_directory: &Path, ngram: NgramOptions) -> Result<Self, SearchError> {
        let root = data_directory.join(INDEX_DIRECTORY);
        fs::create_dir_all(&root)?;
        let current = match open_current(&root) {
            Ok(generation) => Current::Open(Arc::new(generation)),
            Err(reason) => Current::Unusable(reason),
        };
        let keep = match &current {
            Current::Open(generation) => Some(generation.number),
            Current::Unusable(_) => None,
        };
        remove_other_generations(&root, keep)?;
        Ok(Self {
            root,
            ngram,
            current: RwLock::new(current),
        })
    }

    /// Why the current generation must be replaced, if it must.
    pub fn rebuild_reason(&self) -> Option<RebuildReason> {
        match &*self
            .current
            .read()
            .expect("the current generation is replaced whole")
        {
            Current::Open(generation) if generation.ngram != self.ngram => {
                Some(RebuildReason::NgramChanged)
            }
            Current::Open(_) => None,
            Current::Unusable(reason) => Some(reason.clone()),
        }
    }

    /// The generation searches and updates use.
    ///
    /// # Errors
    /// Fails when there is none until a rebuild.
    pub fn current(&self) -> Result<Arc<IndexGeneration>, SearchError> {
        match &*self
            .current
            .read()
            .expect("the current generation is replaced whole")
        {
            Current::Open(generation) => Ok(Arc::clone(generation)),
            Current::Unusable(reason) => Err(SearchError::Unavailable(reason.clone())),
        }
    }

    /// An empty generation with the configured n-gram settings, which becomes current only on
    /// [`SearchIndex::switch`]; one dropped instead is removed when the index is next opened.
    ///
    /// # Errors
    /// Fails when its directory cannot be replaced or the index created.
    pub fn new_generation(&self) -> Result<IndexGeneration, SearchError> {
        let number = match &*self
            .current
            .read()
            .expect("the current generation is replaced whole")
        {
            Current::Open(generation) => generation.number + 1,
            Current::Unusable(_) => 1,
        };
        IndexGeneration::create(&self.root.join(number.to_string()), number, self.ngram)
    }

    /// Makes `generation` current, durably, then removes the one it replaces.
    ///
    /// # Errors
    /// Fails when the manifest cannot be written; the current generation then stays current.
    pub fn switch(&self, generation: IndexGeneration) -> Result<(), SearchError> {
        write_manifest(&self.root, &generation)?;
        let replaced = std::mem::replace(
            &mut *self
                .current
                .write()
                .expect("the current generation is replaced whole"),
            Current::Open(Arc::new(generation)),
        );
        if let Current::Open(replaced) = replaced {
            let path = self.root.join(replaced.number.to_string());
            drop(replaced);
            // Searches may still read the replaced generation; where its files cannot be removed
            // while open, the next `open` removes them.
            let _ = fs::remove_dir_all(path);
        }
        Ok(())
    }

    /// The entities matching `query`, most relevant first, in the current generation as it is
    /// now. Invalid syntax matches nothing.
    ///
    /// # Errors
    /// Fails when the index cannot be read, or there is no usable generation.
    pub fn hits(&self, entity: SearchEntity, query: &str) -> Result<SearchHits, SearchError> {
        self.current()?.hits(entity, query)
    }
}

fn open_current(root: &Path) -> Result<IndexGeneration, RebuildReason> {
    let unreadable = |error: &dyn fmt::Display| RebuildReason::Unreadable(error.to_string());
    let manifest = match fs::read(root.join(MANIFEST)) {
        Ok(manifest) => manifest,
        Err(error) if error.kind() == io::ErrorKind::NotFound => {
            return Err(RebuildReason::Missing);
        }
        Err(error) => return Err(unreadable(&error)),
    };
    let manifest: Manifest =
        serde_json::from_slice(&manifest).map_err(|error| unreadable(&error))?;
    if manifest.format != FORMAT {
        return Err(RebuildReason::OtherFormat);
    }
    let ngram = NgramOptions::new(
        manifest.min_gram,
        manifest.max_gram,
        manifest.preserve_original,
    )
    .ok_or_else(|| RebuildReason::Unreadable("invalid n-gram settings".to_owned()))?;
    let path = root.join(manifest.generation.to_string());
    IndexGeneration::open(&path, manifest.generation, ngram).map_err(|error| unreadable(&error))
}

fn remove_other_generations(root: &Path, keep: Option<u64>) -> io::Result<()> {
    let keep = keep.map(|number| number.to_string());
    for entry in fs::read_dir(root)? {
        let entry = entry?;
        let name = entry.file_name();
        if name == MANIFEST || keep.as_deref().is_some_and(|keep| name == keep) {
            continue;
        }
        if entry.file_type()?.is_dir() {
            fs::remove_dir_all(entry.path())?;
        } else {
            fs::remove_file(entry.path())?;
        }
    }
    Ok(())
}

/// Replaces the manifest so that a crash leaves either the old or the new one.
fn write_manifest(root: &Path, generation: &IndexGeneration) -> io::Result<()> {
    let ngram = generation.ngram;
    let manifest = Manifest {
        format: FORMAT,
        generation: generation.number,
        min_gram: ngram.min_gram,
        max_gram: ngram.max_gram,
        preserve_original: ngram.preserve_original,
    };
    let temporary = root.join(MANIFEST_TEMPORARY);
    let mut file = File::create(&temporary)?;
    file.write_all(&serde_json::to_vec(&manifest).map_err(io::Error::other)?)?;
    file.sync_all()?;
    generation::retry_permission_denied(
        || fs::rename(&temporary, root.join(MANIFEST)),
        |error| error.kind() == io::ErrorKind::PermissionDenied,
    )?;
    #[cfg(unix)]
    File::open(root)?.sync_all()?;
    Ok(())
}
