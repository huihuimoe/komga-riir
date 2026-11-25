//! Logging to standard output and to a rolling file, with the baseline's logback rolling policy:
//! 10 MB files archived as gzip, archives kept for 7 days and 1 GB in total, cleaned at startup.

use std::fs::{self, File, OpenOptions};
use std::io::{self, BufReader, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, mpsc};
use std::thread;
use std::time::{Duration, SystemTime};

use flate2::Compression;
use flate2::write::GzEncoder;
use tracing_subscriber::EnvFilter;
use tracing_subscriber::layer::SubscriberExt;
use tracing_subscriber::util::SubscriberInitExt;

/// Installs the global subscriber; `RUST_LOG` overrides the `info` level.
pub fn init(log_file: &Path) -> io::Result<()> {
    // Absolute, so a bare file name still has a directory to list archives in.
    let file = RollingFile::open(std::path::absolute(log_file)?, Limits::BASELINE)?;
    tracing_subscriber::registry()
        .with(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .with(tracing_subscriber::fmt::layer())
        .with(
            tracing_subscriber::fmt::layer()
                .with_ansi(false)
                .with_writer(Mutex::new(file)),
        )
        .init();
    Ok(())
}

#[derive(Clone, Copy)]
struct Limits {
    max_file_size: u64,
    max_history: Duration,
    total_size_cap: u64,
}

impl Limits {
    const BASELINE: Self = Self {
        max_file_size: 10 * 1024 * 1024,
        max_history: Duration::from_secs(7 * 24 * 60 * 60),
        total_size_cap: 1024 * 1024 * 1024,
    };
}

enum ArchiveCommand {
    Archive {
        pending: PathBuf,
    },
    #[cfg(test)]
    Sync(mpsc::Sender<()>),
}

/// Appends to `path`; once it would exceed the size limit, renames the active file immediately
/// under the lock and hands off gzip compression and archive pruning to a dedicated background
/// thread so calling threads (including Tokio workers) never block on compression.
struct RollingFile {
    path: PathBuf,
    file: Option<File>,
    size: u64,
    roll_seq: u64,
    limits: Limits,
    archiver: mpsc::Sender<ArchiveCommand>,
}

impl RollingFile {
    fn open(path: PathBuf, limits: Limits) -> io::Result<Self> {
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let file = OpenOptions::new().create(true).append(true).open(&path)?;
        let size = file.metadata()?.len();
        clean_archives(&path, size, limits)?;
        let archiver = spawn_archiver(path.clone(), limits)?;
        Ok(Self {
            path,
            file: Some(file),
            size,
            roll_seq: 0,
            limits,
            archiver,
        })
    }

    /// Rotates the active log file quickly under the mutex: closes the handle (required for
    /// atomic rename on Windows), renames it to a temporary pending file, opens a fresh active
    /// log file, and enqueues background compression.
    fn roll(&mut self) -> io::Result<()> {
        if let Some(file) = self.file.as_mut() {
            file.flush()?;
        }
        let pending = self.next_pending_path();
        // Drop the open handle before renaming so Windows does not reject with a sharing violation.
        drop(self.file.take());
        if let Err(error) = fs::rename(&self.path, &pending) {
            self.file = Some(
                OpenOptions::new()
                    .create(true)
                    .append(true)
                    .open(&self.path)?,
            );
            return Err(error);
        }
        self.file = Some(File::create(&self.path)?);
        self.size = 0;
        let _ = self.archiver.send(ArchiveCommand::Archive { pending });
        Ok(())
    }

    fn next_pending_path(&mut self) -> PathBuf {
        loop {
            let seq = self.roll_seq;
            self.roll_seq = self.roll_seq.wrapping_add(1);
            let mut name = self.path.file_name().unwrap_or_default().to_os_string();
            name.push(format!(".rolling.{seq}.tmp"));
            let candidate = self.path.with_file_name(name);
            if !candidate.exists() {
                return candidate;
            }
        }
    }

    #[cfg(test)]
    fn sync_archives(&self) {
        let (tx, rx) = mpsc::channel();
        let _ = self.archiver.send(ArchiveCommand::Sync(tx));
        let _ = rx.recv();
    }
}

fn spawn_archiver(path: PathBuf, limits: Limits) -> io::Result<mpsc::Sender<ArchiveCommand>> {
    let (tx, rx) = mpsc::channel::<ArchiveCommand>();
    thread::Builder::new()
        .name("komga-log-archiver".to_owned())
        .spawn(move || {
            for command in rx {
                match command {
                    ArchiveCommand::Archive { pending } => {
                        if let Err(error) = compress_and_clean(&path, &pending, limits) {
                            eprintln!(
                                "Failed to archive rotated log {}: {error}",
                                pending.display()
                            );
                        }
                    }
                    #[cfg(test)]
                    ArchiveCommand::Sync(done) => {
                        let _ = done.send(());
                    }
                }
            }
        })?;
    Ok(tx)
}

fn compress_and_clean(log_path: &Path, pending: &Path, limits: Limits) -> io::Result<()> {
    let archive = next_archive(log_path);
    let mut encoder = GzEncoder::new(File::create_new(&archive)?, Compression::default());
    io::copy(&mut BufReader::new(File::open(pending)?), &mut encoder)?;
    encoder.finish()?.sync_all()?;
    fs::remove_file(pending)?;
    let active_size = fs::metadata(log_path).map_or(0, |meta| meta.len());
    clean_archives(log_path, active_size, limits)
}

fn next_archive(log_path: &Path) -> PathBuf {
    let date = jiff::ZonedDateTime::now().strftime("%Y-%m-%d").to_string();
    let mut name = log_path.file_name().unwrap_or_default().to_os_string();
    name.push(format!(".{date}."));
    (0..)
        .map(|index| {
            let mut name = name.clone();
            name.push(format!("{index}.gz"));
            log_path.with_file_name(name)
        })
        .find(|archive| !archive.exists())
        .expect("some archive index is free")
}

/// Deletes archives older than the history limit, then the oldest until the total fits.
fn clean_archives(log_path: &Path, active_size: u64, limits: Limits) -> io::Result<()> {
    let mut archives = list_archives(log_path)?;
    archives.sort_by_key(|archive| std::cmp::Reverse(archive.modified));
    let now = SystemTime::now();
    let mut total = active_size;
    for archive in archives {
        total += archive.size;
        let expired = now
            .duration_since(archive.modified)
            .is_ok_and(|age| age > limits.max_history);
        if expired || total > limits.total_size_cap {
            fs::remove_file(&archive.path)?;
        }
    }
    Ok(())
}

fn list_archives(log_path: &Path) -> io::Result<Vec<Archive>> {
    let Some(directory) = log_path.parent() else {
        return Ok(Vec::new());
    };
    let prefix = format!("{}.", log_path.file_name().unwrap_or_default().display());
    let mut archives = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        let name = entry.file_name();
        let name = name.to_string_lossy();
        if name.starts_with(&prefix) && name.ends_with(".gz") {
            let metadata = entry.metadata()?;
            archives.push(Archive {
                path: entry.path(),
                size: metadata.len(),
                modified: metadata.modified()?,
            });
        }
    }
    Ok(archives)
}

struct Archive {
    path: PathBuf,
    size: u64,
    modified: SystemTime,
}

impl Write for RollingFile {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if self.size > 0 && self.size + buf.len() as u64 > self.limits.max_file_size {
            self.roll()?;
        }
        let file = self
            .file
            .as_mut()
            .ok_or_else(|| io::Error::other("Log file handle is unavailable"))?;
        let written = file.write(buf)?;
        self.size += written as u64;
        Ok(written)
    }

    fn flush(&mut self) -> io::Result<()> {
        match self.file.as_mut() {
            Some(file) => file.flush(),
            None => Ok(()),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use flate2::read::GzDecoder;

    use super::*;

    const SMALL: Limits = Limits {
        max_file_size: 10,
        max_history: Limits::BASELINE.max_history,
        total_size_cap: 100,
    };

    fn names(directory: &Path) -> Vec<String> {
        let mut names: Vec<_> = fs::read_dir(directory)
            .expect("Log directory")
            .map(|entry| {
                entry
                    .expect("Entry")
                    .file_name()
                    .to_string_lossy()
                    .into_owned()
            })
            .collect();
        names.sort();
        names
    }

    #[test]
    fn a_full_file_is_archived_as_gzip_and_restarted() {
        let directory = komga_test_support::tempdir().expect("Temporary directory");
        let path = directory.path().join("logs").join("komga.log");
        let mut file = RollingFile::open(path.clone(), SMALL).expect("Open log");
        file.write_all(b"first\n").expect("Write");
        file.write_all(b"second\n").expect("Write");
        file.sync_archives();

        let date = jiff::ZonedDateTime::now().strftime("%Y-%m-%d").to_string();
        let archive = format!("komga.log.{date}.0.gz");
        assert_eq!(
            names(&directory.path().join("logs")),
            ["komga.log", &archive]
        );
        let mut archived = String::new();
        GzDecoder::new(File::open(path.with_file_name(&archive)).expect("Archive"))
            .read_to_string(&mut archived)
            .expect("Gzip");
        assert_eq!(archived, "first\n");
        assert_eq!(fs::read_to_string(&path).expect("Log"), "second\n");
    }

    #[test]
    fn opening_removes_expired_archives_then_the_oldest_over_the_cap() {
        let directory = komga_test_support::tempdir().expect("Temporary directory");
        let now = SystemTime::now();
        let hour = Duration::from_secs(3600);
        for (name, age) in [
            ("komga.log.a.0.gz", hour),
            ("komga.log.b.0.gz", 2 * hour),
            ("komga.log.c.0.gz", 3 * hour),
            ("komga.log.d.0.gz", Limits::BASELINE.max_history + hour),
            ("other.log.e.0.gz", Limits::BASELINE.max_history + hour),
        ] {
            let file = File::create(directory.path().join(name)).expect("Archive");
            file.set_len(40).expect("Size");
            file.set_modified(now - age).expect("Modified");
        }

        RollingFile::open(directory.path().join("komga.log"), SMALL).expect("Open log");

        assert_eq!(
            names(directory.path()),
            [
                "komga.log",
                "komga.log.a.0.gz",
                "komga.log.b.0.gz",
                "other.log.e.0.gz"
            ]
        );
    }
}
