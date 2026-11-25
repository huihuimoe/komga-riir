//! Book files as one ZIP archive written while it is sent, as Kotlin's read list and series
//! downloads stream them.

use std::io;
use std::path::PathBuf;

use jiff::ZonedDateTime;
use komga_archive::zip::writer::ZipStreamWriter;
use tokio::fs::{self, File};
use tokio::io::AsyncReadExt;
use tokio::sync::mpsc::{self, Receiver, Sender};
use tracing::{debug, warn};

/// Bytes read from a book file at once.
const CHUNK: usize = 64 * 1024;
/// Archive chunks waiting for the client; a slow client holds back the writing.
const QUEUED_CHUNKS: usize = 4;

/// An archive being written.
pub struct BookArchive {
    /// The archive's own name, ending in `.zip`.
    pub file_name: String,
    /// The archive in order. An error ends it early: a file failed to read, so the archive is
    /// incomplete.
    pub chunks: Receiver<io::Result<Vec<u8>>>,
}

impl BookArchive {
    /// Starts writing `entries`, each an entry name and the file it holds, in order. Missing
    /// files are skipped.
    pub(crate) fn spawn(file_name: String, entries: Vec<(String, PathBuf)>) -> Self {
        let (sender, chunks) = mpsc::channel(QUEUED_CHUNKS);
        tokio::spawn(async move {
            if let Err(error) = write(entries, &sender).await {
                // Aborts the response; fails only when the client is already gone.
                let _ = sender.send(Err(error)).await;
            }
        });
        Self { file_name, chunks }
    }
}

async fn write(
    entries: Vec<(String, PathBuf)>,
    sender: &Sender<io::Result<Vec<u8>>>,
) -> io::Result<()> {
    let mut zip = ZipStreamWriter::new(ZonedDateTime::now().datetime());
    let mut buffer = vec![0; CHUNK];
    for (entry, path) in entries {
        let mut file = match File::open(&path).await {
            Ok(file) => file,
            Err(_) if !fs::try_exists(&path).await.unwrap_or(false) => {
                warn!(path = %path.display(), "Book file not found, skipping archive entry");
                continue;
            }
            Err(error) => return Err(error),
        };
        debug!(path = %path.display(), "Adding file to zip archive");
        send(sender, zip.start_entry(&entry)).await?;
        loop {
            let read = file.read(&mut buffer).await?;
            if read == 0 {
                break;
            }
            send(sender, zip.write(&buffer[..read])).await?;
        }
        send(sender, zip.finish_entry()).await?;
    }
    send(sender, zip.finish()).await
}

/// Sends `bytes` unless empty; fails once the client is gone.
async fn send(sender: &Sender<io::Result<Vec<u8>>>, bytes: Vec<u8>) -> io::Result<()> {
    if bytes.is_empty() {
        return Ok(());
    }
    sender
        .send(Ok(bytes))
        .await
        .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))
}
