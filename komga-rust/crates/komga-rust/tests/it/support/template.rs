//! Cross-process cached directory templates under `<target>/tmp/komga-rust-test/templates/`.
//!
//! Because `cargo nextest` runs each test in its own process, an in-process `OnceLock` cannot share
//! expensive cold-start fixtures across tests. This module builds each template once into a
//! fingerprint-keyed directory guarded by an OS file lock.

use std::fs::{self, File, OpenOptions, TryLockError};
use std::future::Future;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crc32fast::Hasher;
use tokio::time;

const READY_MARKER: &str = ".ready";

/// Returns the ready template directory for `name`, running `populate` at most once across all test
/// processes on this machine.
pub async fn cached_dir<F, Fut>(name: &str, populate: F) -> PathBuf
where
    F: FnOnce(PathBuf) -> Fut,
    Fut: Future<Output = ()>,
{
    let templates = komga_test_support::root().join("templates");
    let dir = templates.join(name);
    let ready = dir.join(READY_MARKER);
    if ready.exists() {
        return dir;
    }

    fs::create_dir_all(&templates).expect("Create template cache root");
    let _lock = acquire_lock(&templates.join(format!("{name}.lock"))).await;
    if ready.exists() {
        return dir;
    }

    if dir.exists() {
        fs::remove_dir_all(&dir).expect("Clear incomplete template");
    }
    fs::create_dir_all(&dir).expect("Create template directory");
    populate(dir.clone()).await;
    fs::write(&ready, b"ok").expect("Write template ready marker");
    dir
}

/// CRC32 fingerprint over all SQL migration files (`main`, `tasks`, `era`) so template caches
/// invalidate automatically whenever any migration is added or edited.
pub fn migrations_fingerprint() -> u32 {
    let mut hasher = Hasher::new();
    hash_migrations(&mut hasher);
    hasher.finalize()
}

/// CRC32 fingerprint for the claimed-server template: covers SQL migrations, the Tantivy search
/// schema/format, and the test admin credentials.
pub fn claimed_server_fingerprint(admin_email: &str, admin_password: &str) -> u32 {
    let mut hasher = Hasher::new();
    hash_migrations(&mut hasher);
    let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("..");
    for relative in [
        "komga-search/src/schema.rs",
        "komga-search/src/index/mod.rs",
    ] {
        hasher.update(&fs::read(root.join(relative)).expect("Read search index source"));
    }
    hasher.update(admin_email.as_bytes());
    hasher.update(b"\0");
    hasher.update(admin_password.as_bytes());
    hasher.finalize()
}

/// Recursively copies `source` into `destination`, skipping template metadata and lock files.
pub fn copy_into(source: &Path, destination: &Path) {
    fs::create_dir_all(destination).expect("Create destination directory");
    for entry in fs::read_dir(source).expect("Read template directory") {
        let entry = entry.expect("Template entry");
        let name = entry.file_name();
        if name == READY_MARKER || name == ".komga.lock" {
            continue;
        }
        let target = destination.join(&name);
        if entry.file_type().expect("Entry file type").is_dir() {
            copy_into(&entry.path(), &target);
        } else {
            fs::copy(entry.path(), target).expect("Copy template file");
        }
    }
}

fn hash_migrations(hasher: &mut Hasher) {
    let base = Path::new(env!("CARGO_MANIFEST_DIR")).join("../komga-db/migrations");
    let mut paths = Vec::new();
    for subdir in ["main", "tasks", "era"] {
        for entry in fs::read_dir(base.join(subdir)).expect("Read migrations directory") {
            paths.push(entry.expect("Migration entry").path());
        }
    }
    paths.sort();

    for path in paths {
        if let Some(name) = path.file_name().and_then(|n| n.to_str()) {
            hasher.update(name.as_bytes());
        }
        hasher.update(&fs::read(&path).expect("Read migration file"));
    }
}

async fn acquire_lock(path: &Path) -> File {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .expect("Open template lock file");
    loop {
        match file.try_lock() {
            Ok(()) => return file,
            Err(TryLockError::WouldBlock) => time::sleep(Duration::from_millis(20)).await,
            Err(TryLockError::Error(error)) => panic!("Lock template {}: {error}", path.display()),
        }
    }
}
