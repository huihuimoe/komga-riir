//! Readiness checks behind `/actuator/health`, after Spring Boot's `db` and `diskSpace`
//! indicators.

use std::path::PathBuf;

use komga_db::Database;

/// Spring Boot's default `management.health.diskspace.threshold`.
pub const DISK_SPACE_THRESHOLD: u64 = 10 * 1024 * 1024;

/// Checks the resources requests need, without writing anything.
#[derive(Clone)]
pub struct HealthService {
    database: Database,
    tasks: Database,
    data_directory: PathBuf,
}

/// The result of one round of checks.
#[derive(Debug)]
pub struct Health {
    pub database: bool,
    pub tasks_database: bool,
    pub disk: DiskSpace,
}

/// Space on the file system holding the databases, as `java.io.File` reports it: zero when
/// it cannot be read.
#[derive(Debug)]
pub struct DiskSpace {
    pub path: PathBuf,
    pub exists: bool,
    pub total: u64,
    pub free: u64,
}

impl HealthService {
    #[must_use]
    pub fn new(database: Database, tasks: Database, data_directory: PathBuf) -> Self {
        Self {
            database,
            tasks,
            data_directory,
        }
    }

    pub async fn check(&self) -> Health {
        let (database, tasks_database) = tokio::join!(self.database.ping(), self.tasks.ping());
        let path = self.data_directory.clone();
        let disk = tokio::task::spawn_blocking(move || disk_space(path))
            .await
            .expect("Reading disk space must not panic");
        Health {
            database: database.is_ok(),
            tasks_database: tasks_database.is_ok(),
            disk,
        }
    }
}

impl Health {
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.database && self.tasks_database && self.disk.is_up()
    }
}

impl DiskSpace {
    #[must_use]
    pub fn is_up(&self) -> bool {
        self.free >= DISK_SPACE_THRESHOLD
    }
}

fn disk_space(path: PathBuf) -> DiskSpace {
    let exists = path.exists();
    // On Windows, `fs4::statvfs` resolves the volume root from the path prefix even when the
    // directory is missing; match `java.io.File` by reporting zero unless the path exists.
    let (total, free) = if exists {
        fs4::statvfs(&path)
            .map(|stats| (stats.total_space(), stats.available_space()))
            .unwrap_or_default()
    } else {
        (0, 0)
    };
    DiskSpace {
        path,
        exists,
        total,
        free,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    async fn database(directory: &std::path::Path, name: &str) -> Database {
        Database::connect(&directory.join(name))
            .await
            .expect("Database")
    }

    #[tokio::test]
    async fn is_up_while_the_databases_and_disk_space_are_available() {
        let directory = komga_test_support::tempdir().expect("Temp dir");
        let main = database(directory.path(), "main.sqlite").await;
        let tasks = database(directory.path(), "tasks.sqlite").await;
        let health = HealthService::new(main, tasks.clone(), directory.path().to_path_buf());

        let up = health.check().await;
        assert!(up.is_up(), "{up:?}");
        assert!(up.disk.exists && up.disk.total >= up.disk.free);

        tasks.close().await;
        let down = health.check().await;
        assert!(down.database && !down.tasks_database && !down.is_up());
    }

    #[tokio::test]
    async fn is_down_when_the_data_directory_cannot_be_read() {
        let directory = komga_test_support::tempdir().expect("Temp dir");
        let main = database(directory.path(), "main.sqlite").await;
        let tasks = database(directory.path(), "tasks.sqlite").await;
        let missing = directory.path().join("missing");
        let health = HealthService::new(main, tasks, missing).check().await;

        assert!(!health.disk.exists && health.disk.free == 0 && !health.is_up());
    }
}
