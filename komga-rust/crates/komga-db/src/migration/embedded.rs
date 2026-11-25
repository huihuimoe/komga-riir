//! SQL migrations embedded from `migrations/` in Flyway's naming, normalized once on first use.

use std::sync::LazyLock;

use rust_embed::RustEmbed;

use super::checksum::flyway_checksum;

#[derive(RustEmbed)]
#[folder = "migrations/main/"]
struct MainFiles;

#[derive(RustEmbed)]
#[folder = "migrations/tasks/"]
struct TasksFiles;

#[derive(RustEmbed)]
#[folder = "migrations/era/"]
struct EraFiles;

pub(super) struct SqlMigration {
    pub(super) version: String,
    pub(super) description: String,
    pub(super) script: String,
    pub(super) sql: String,
    pub(super) checksum: i32,
}

pub(super) static MAIN: LazyLock<Vec<SqlMigration>> = LazyLock::new(load::<MainFiles>);
pub(super) static TASKS: LazyLock<Vec<SqlMigration>> = LazyLock::new(load::<TasksFiles>);
pub(super) static ERA: LazyLock<Vec<SqlMigration>> = LazyLock::new(load::<EraFiles>);

/// Embedded files are fixed at build time, so a malformed name or non-UTF-8 script is a packaging bug.
fn load<Files: RustEmbed>() -> Vec<SqlMigration> {
    let mut names: Vec<_> = Files::iter().collect();
    names.sort();
    names
        .into_iter()
        .map(|name| {
            let file = Files::get(&name).expect("listed migration is embedded");
            let (version, description) = parse_flyway_name(&name);
            let content = std::str::from_utf8(&file.data)
                .unwrap_or_else(|error| panic!("{name} is not valid UTF-8: {error}"));
            SqlMigration {
                version,
                description,
                sql: replace_flyway_placeholders(content),
                checksum: flyway_checksum(&file.data),
                script: name.into_owned(),
            }
        })
        .collect()
}

fn parse_flyway_name(file_name: &str) -> (String, String) {
    let (version, description) = file_name
        .strip_suffix(".sql")
        .and_then(|base| base.split_once("__"))
        .and_then(|(version, description)| Some((version.strip_prefix('V')?, description)))
        .unwrap_or_else(|| panic!("unexpected Flyway migration name: {file_name}"));
    (version.to_owned(), description.replace('_', " "))
}

fn replace_flyway_placeholders(content: &str) -> String {
    [
        ("${delete-empty-collections}", "1"),
        ("${delete-empty-read-lists}", "1"),
        ("${library-file-hashing}", "1"),
        ("${library-scan-startup}", "0"),
    ]
    .into_iter()
    .fold(content.to_owned(), |acc, (from, to)| acc.replace(from, to))
}
