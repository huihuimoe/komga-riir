use super::super::error::DbError;
use super::embedded::SqlMigration;
use super::{Migration, history};

pub(super) fn validate_history(
    entries: &[history::Entry],
    sql_migrations: &[SqlMigration],
    jdbc_migrations: &[history::JdbcMigration],
    require_kotlin_floor: bool,
) -> Result<(), DbError> {
    for entry in entries {
        validate_entry(entry, sql_migrations, jdbc_migrations)?;
    }

    if require_kotlin_floor {
        validate_kotlin_floor(entries, jdbc_migrations)?;
    }

    validate_no_gaps(entries, sql_migrations, jdbc_migrations)
}

fn validate_entry(
    entry: &history::Entry,
    sql_migrations: &[SqlMigration],
    jdbc_migrations: &[history::JdbcMigration],
) -> Result<(), DbError> {
    let Some(version) = entry.version.as_deref() else {
        return Err(DbError::UnsupportedSchema(
            "migration history contains an unversioned entry".to_owned(),
        ));
    };
    if !entry.success {
        return Err(DbError::FailedMigration(version.to_owned()));
    }

    let migration = find_migration(version, sql_migrations, jdbc_migrations)
        .ok_or_else(|| DbError::UnknownVersion(version.to_owned()))?;
    if entry.migration_type != migration.migration_type() {
        return Err(DbError::UnsupportedSchema(format!(
            "migration {version} has type `{}`, expected `{}`",
            entry.migration_type,
            migration.migration_type()
        )));
    }
    if let Some(expected) = migration.checksum()
        && entry.checksum != Some(expected)
    {
        return Err(DbError::ChecksumMismatch {
            version: version.to_owned(),
            expected,
            actual: entry.checksum,
        });
    }
    Ok(())
}

fn validate_kotlin_floor(
    entries: &[history::Entry],
    jdbc_migrations: &[history::JdbcMigration],
) -> Result<(), DbError> {
    let latest_version = entries
        .iter()
        .filter_map(|entry| entry.version.as_deref())
        .max();
    if latest_version.is_none_or(|version| version < history::MIN_SUPPORTED_KOTLIN_VERSION) {
        return Err(DbError::UnsupportedSchema(format!(
            "source schema is below the supported version {}; upgrade it with Kotlin Komga first",
            history::MIN_SUPPORTED_KOTLIN_VERSION
        )));
    }

    for migration in jdbc_migrations {
        let applied = entries.iter().any(|entry| {
            entry.version.as_deref() == Some(migration.version)
                && entry.migration_type == "JDBC"
                && entry.success
        });
        if !applied {
            return Err(DbError::UnsupportedSchema(format!(
                "source schema is missing successful Kotlin JDBC migration {}; upgrade it with Kotlin Komga first",
                migration.version
            )));
        }
    }
    Ok(())
}

fn validate_no_gaps(
    entries: &[history::Entry],
    sql_migrations: &[SqlMigration],
    jdbc_migrations: &[history::JdbcMigration],
) -> Result<(), DbError> {
    let Some(latest_version) = entries
        .iter()
        .filter_map(|entry| entry.version.as_deref())
        .max()
    else {
        return Ok(());
    };

    let mut sql_index = 0;
    let mut jdbc_index = 0;
    while let Some(migration) = next_migration(
        sql_migrations,
        &mut sql_index,
        jdbc_migrations,
        &mut jdbc_index,
    ) {
        if migration.version() > latest_version {
            break;
        }
        if !entries
            .iter()
            .any(|entry| entry.version.as_deref() == Some(migration.version()) && entry.success)
        {
            return Err(DbError::UnsupportedSchema(format!(
                "migration history is missing successful version {}",
                migration.version()
            )));
        }
    }
    Ok(())
}

pub(super) fn has_pending_migrations(
    entries: &[history::Entry],
    sql_migrations: &[SqlMigration],
    jdbc_migrations: &[history::JdbcMigration],
) -> bool {
    sql_migrations.iter().any(|migration| {
        !entries.iter().any(|entry| {
            entry.version.as_deref() == Some(migration.version.as_str()) && entry.success
        })
    }) || jdbc_migrations.iter().any(|migration| {
        !entries
            .iter()
            .any(|entry| entry.version.as_deref() == Some(migration.version) && entry.success)
    })
}

fn find_migration<'a>(
    version: &str,
    sql_migrations: &'a [SqlMigration],
    jdbc_migrations: &'a [history::JdbcMigration],
) -> Option<Migration<'a>> {
    sql_migrations
        .iter()
        .find(|migration| migration.version == version)
        .map(Migration::Sql)
        .or_else(|| {
            jdbc_migrations
                .iter()
                .find(|migration| migration.version == version)
                .map(Migration::Jdbc)
        })
}

pub(super) fn next_migration<'a>(
    sql_migrations: &'a [SqlMigration],
    sql_index: &mut usize,
    jdbc_migrations: &'a [history::JdbcMigration],
    jdbc_index: &mut usize,
) -> Option<Migration<'a>> {
    match (
        sql_migrations.get(*sql_index),
        jdbc_migrations.get(*jdbc_index),
    ) {
        (Some(sql), Some(jdbc)) if sql.version.as_str() <= jdbc.version => {
            *sql_index += 1;
            Some(Migration::Sql(sql))
        }
        (Some(_), Some(jdbc)) => {
            *jdbc_index += 1;
            Some(Migration::Jdbc(jdbc))
        }
        (Some(sql), None) => {
            *sql_index += 1;
            Some(Migration::Sql(sql))
        }
        (None, Some(jdbc)) => {
            *jdbc_index += 1;
            Some(Migration::Jdbc(jdbc))
        }
        (None, None) => None,
    }
}
