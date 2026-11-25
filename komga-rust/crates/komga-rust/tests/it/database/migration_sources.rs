use std::ffi::OsStr;
use std::fs;
use std::path::{Path, PathBuf};

fn sql_files(directory: &Path) -> Vec<PathBuf> {
    let mut files = fs::read_dir(directory)
        .unwrap_or_else(|error| panic!("Failed to list {}: {error}", directory.display()))
        .map(|entry| {
            entry
                .expect("Failed to read migration directory entry")
                .path()
        })
        .filter(|path| path.extension() == Some(OsStr::new("sql")))
        .collect::<Vec<_>>();
    files.sort();
    files
}

fn assert_migrations_match(source: &Path, target: &Path) {
    let source_files = sql_files(source);
    assert!(
        !source_files.is_empty(),
        "{} contains no SQL migrations",
        source.display()
    );

    for source_file in source_files {
        let file_name = source_file
            .file_name()
            .expect("Migration source must have a filename");
        let target_file = target.join(file_name);
        let source_bytes = fs::read(&source_file)
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", source_file.display()));
        let target_bytes = fs::read(&target_file)
            .unwrap_or_else(|error| panic!("Failed to read {}: {error}", target_file.display()));
        assert!(
            source_bytes == target_bytes,
            "{} differs byte-for-byte from Kotlin source {}",
            target_file.display(),
            source_file.display()
        );
    }
}

#[test]
fn test_embedded_sql_migrations_match_kotlin_sources() {
    let repo_root = Path::new(env!("CARGO_MANIFEST_DIR")).join("../../..");
    assert_migrations_match(
        &repo_root.join("komga/src/flyway/resources/db/migration/sqlite"),
        &repo_root.join("komga-rust/crates/komga-db/migrations/main"),
    );
    assert_migrations_match(
        &repo_root.join("komga/src/flyway/resources/tasks/migration/sqlite"),
        &repo_root.join("komga-rust/crates/komga-db/migrations/tasks"),
    );
}
