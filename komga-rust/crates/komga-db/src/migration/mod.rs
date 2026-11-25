//! Flyway-compatible migration embedding and execution.

use std::path::Path;

use sqlx::pool::PoolConnection;
use sqlx::{Pool, Sqlite};
use tracing::info;

use super::error::DbError;

mod backup;
mod checksum;
mod embedded;
mod era_schema;
mod history;
#[cfg(test)]
mod iden_sync;
mod komga_rust_legacy;
mod runner;
mod validate;

use embedded::SqlMigration;
use runner::{record, run_flyway_migrations};
use validate::{has_pending_migrations, next_migration, validate_history};

#[derive(Clone, Copy)]
enum Migration<'a> {
    Sql(&'a SqlMigration),
    Jdbc(&'a history::JdbcMigration),
}

impl<'a> Migration<'a> {
    fn version(self) -> &'a str {
        match self {
            Self::Sql(migration) => &migration.version,
            Self::Jdbc(migration) => migration.version,
        }
    }

    fn description(self) -> &'a str {
        match self {
            Self::Sql(migration) => &migration.description,
            Self::Jdbc(migration) => migration.description,
        }
    }

    fn script(self) -> &'a str {
        match self {
            Self::Sql(migration) => &migration.script,
            Self::Jdbc(migration) => migration.script,
        }
    }

    fn migration_type(self) -> &'static str {
        match self {
            Self::Sql(_) => "SQL",
            Self::Jdbc(_) => "JDBC",
        }
    }

    fn checksum(self) -> Option<i32> {
        match self {
            Self::Sql(migration) => Some(migration.checksum),
            Self::Jdbc(_) => None,
        }
    }

    fn sql(self) -> Option<&'a str> {
        match self {
            Self::Sql(migration) => Some(&migration.sql),
            Self::Jdbc(_) => None,
        }
    }
}

/// The schema a database belongs to, and its file name inside a backup set.
#[derive(Clone, Copy)]
pub(crate) enum Schema {
    Main,
    Tasks,
}

impl Schema {
    fn sql_migrations(self) -> &'static [SqlMigration] {
        match self {
            Self::Main => &embedded::MAIN,
            Self::Tasks => &embedded::TASKS,
        }
    }

    fn jdbc_migrations(self) -> &'static [history::JdbcMigration] {
        match self {
            Self::Main => history::MAIN_JDBC_MIGRATIONS,
            Self::Tasks => &[],
        }
    }

    fn backup_name(self) -> &'static str {
        match self {
            Self::Main => "database.sqlite",
            Self::Tasks => "tasks.sqlite",
        }
    }
}

/// What a database still needs, read and checked against its history without writing.
struct Plan {
    conn: PoolConnection<Sqlite>,
    schema: Schema,
    entries: Vec<history::Entry>,
    era: Vec<&'static SqlMigration>,
    fresh: bool,
    komga_rust: Option<komga_rust_legacy::Adoption>,
}

impl Plan {
    async fn read(pool: &Pool<Sqlite>, schema: Schema) -> Result<Self, DbError> {
        let mut conn = pool.acquire().await?;
        let (snapshot, mut komga_rust) =
            match komga_rust_legacy::read(conn.as_mut(), schema).await? {
                Some((snapshot, adoption)) => (snapshot, Some(adoption)),
                None => (history::load(conn.as_mut()).await?, None),
            };
        validate_history(
            &snapshot.entries,
            schema.sql_migrations(),
            schema.jdbc_migrations(),
            matches!(schema, Schema::Main) && !snapshot.is_fresh,
        )?;
        if let (Some(adoption), Schema::Main) = (&mut komga_rust, schema) {
            adoption.read_file_paths(conn.as_mut()).await?;
        }
        let era = match schema {
            Schema::Main => era_schema::pending(conn.as_mut()).await?,
            Schema::Tasks => Vec::new(),
        };
        Ok(Self {
            conn,
            schema,
            entries: snapshot.entries,
            era,
            fresh: snapshot.is_fresh,
            komga_rust,
        })
    }

    fn changes_existing_data(&self) -> bool {
        !self.fresh
            && (self.komga_rust.is_some()
                || !self.era.is_empty()
                || has_pending_migrations(
                    &self.entries,
                    self.schema.sql_migrations(),
                    self.schema.jdbc_migrations(),
                ))
    }
}

/// Every database is checked before any is written. When one of them would change, all existing
/// ones are first copied into a single backup set under `backups`, so they restore as a pair.
pub(crate) async fn migrate(
    databases: &[(&Pool<Sqlite>, Schema)],
    backups: &Path,
) -> Result<(), DbError> {
    let mut plans = Vec::with_capacity(databases.len());
    for &(pool, schema) in databases {
        plans.push(Plan::read(pool, schema).await?);
    }
    let mut backup = None;
    if plans.iter().any(Plan::changes_existing_data) {
        let mut sources: Vec<_> = plans
            .iter_mut()
            .filter(|plan| !plan.fresh)
            .map(|plan| (plan.schema.backup_name(), plan.conn.as_mut()))
            .collect();
        backup = Some(backup::create(&mut sources, backups).await?);
    }
    apply(plans).await.map_err(|source| match backup {
        Some(backup) => DbError::MigrationAfterBackup {
            backup,
            source: Box::new(source),
        },
        None => source,
    })
}

async fn apply(plans: Vec<Plan>) -> Result<(), DbError> {
    for mut plan in plans {
        if let Some(adoption) = plan.komga_rust.take() {
            adoption.apply(plan.conn.as_mut()).await?;
        }
        run_flyway_migrations(&mut plan).await?;
        match plan.schema {
            Schema::Main => {
                info!("Main database Flyway migrations successfully applied");
                era_schema::apply(plan.conn.as_mut(), &plan.era).await?;
            }
            Schema::Tasks => info!("Tasks database Flyway migrations successfully applied"),
        }
    }
    Ok(())
}
