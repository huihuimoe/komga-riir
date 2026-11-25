use sqlx::SqliteConnection;

use super::super::error::DbError;

pub(super) const MIN_SUPPORTED_KOTLIN_VERSION: &str = "20240422132621";

#[derive(Debug, Clone, Copy)]
pub(super) struct JdbcMigration {
    pub(super) version: &'static str,
    pub(super) description: &'static str,
    pub(super) script: &'static str,
}

pub(super) const MAIN_JDBC_MIGRATIONS: &[JdbcMigration] = &[
    JdbcMigration {
        version: "20200810154730",
        description: "thumbnails part 2",
        script: "db.migration.sqlite.V20200810154730__thumbnails_part_2",
    },
    JdbcMigration {
        version: "20200820150923",
        description: "metadata fields part 2",
        script: "db.migration.sqlite.V20200820150923__metadata_fields_part_2",
    },
    JdbcMigration {
        version: "20210624165023",
        description: "missing series metadata",
        script: "db.migration.sqlite.V20210624165023__missing_series_metadata",
    },
    JdbcMigration {
        version: "20230801104436",
        description: "fix incorrect language codes",
        script: "db.migration.sqlite.V20230801104436__fix_incorrect_language_codes",
    },
    JdbcMigration {
        version: "20240422132621",
        description: "fix read progress locators",
        script: "db.migration.sqlite.V20240422132621__fix_read_progress_locators",
    },
];

#[derive(Debug, sqlx::FromRow)]
pub(super) struct Entry {
    pub(super) version: Option<String>,
    pub(super) migration_type: String,
    pub(super) checksum: Option<i32>,
    pub(super) success: bool,
}

pub(super) struct Snapshot {
    pub(super) entries: Vec<Entry>,
    pub(super) is_fresh: bool,
}

pub(super) async fn load(conn: &mut SqliteConnection) -> Result<Snapshot, DbError> {
    let has_other_objects: bool = sqlx::query_scalar(
        "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name NOT LIKE 'sqlite_%' AND name <> 'flyway_schema_history')",
    )
    .fetch_one(&mut *conn)
    .await?;

    match entries(conn).await? {
        Some(entries) => {
            let is_fresh = entries.is_empty() && !has_other_objects;

            if entries.is_empty() && has_other_objects {
                return Err(DbError::UnsupportedSchema(
                    "database has application objects but no applied Flyway migrations".to_owned(),
                ));
            }

            Ok(Snapshot { entries, is_fresh })
        }
        None if has_other_objects => Err(DbError::UnsupportedSchema(
            "database has application objects but no Flyway migration history".to_owned(),
        )),
        None => Ok(Snapshot {
            entries: Vec::new(),
            is_fresh: true,
        }),
    }
}

/// The recorded migrations in installed order; `None` without a history table.
pub(super) async fn entries(conn: &mut SqliteConnection) -> Result<Option<Vec<Entry>>, DbError> {
    let history_object: Option<String> =
        sqlx::query_scalar("SELECT type FROM sqlite_master WHERE name = 'flyway_schema_history'")
            .fetch_optional(&mut *conn)
            .await?;
    match history_object.as_deref() {
        None => Ok(None),
        Some("table") => Ok(Some(
            sqlx::query_as(
                "SELECT version, type AS migration_type, checksum, success FROM flyway_schema_history ORDER BY installed_rank",
            )
            .fetch_all(&mut *conn)
            .await?,
        )),
        Some(object_type) => Err(DbError::UnsupportedSchema(format!(
            "`flyway_schema_history` is a {object_type}, not a table"
        ))),
    }
}

pub(super) async fn create_table(conn: &mut SqliteConnection) -> Result<(), DbError> {
    sqlx::query(
        r#"
        CREATE TABLE IF NOT EXISTS flyway_schema_history (
            installed_rank INTEGER NOT NULL PRIMARY KEY,
            version VARCHAR(50),
            description VARCHAR(200) NOT NULL,
            type VARCHAR(20) NOT NULL,
            script VARCHAR(1000) NOT NULL,
            checksum INTEGER,
            installed_by VARCHAR(100) NOT NULL,
            installed_on TIMESTAMP NOT NULL DEFAULT CURRENT_TIMESTAMP,
            execution_time INTEGER NOT NULL,
            success BOOLEAN NOT NULL
        );
        "#,
    )
    .execute(&mut *conn)
    .await?;
    Ok(())
}
