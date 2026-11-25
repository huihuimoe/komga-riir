//! Kotlin `ZipExtractor.getEntries`: the members of a ZIP book file, typed from their content.

use std::fs::File;
use std::io::{BufReader, Read};
use std::path::Path;

use komga_archive::media_type;
use komga_archive::zip::{ZipArchive, ZipError};
use komga_types::Dimension;
use tracing::warn;

use super::ContainerEntry;
use super::failure::transient;
use crate::support::natural_order;

/// The file members in natural name order. A member that fails to open or read gets no type;
/// image sizes are read when `analyze_dimensions` is set.
///
/// # Errors
/// The archive cannot be opened, a member needs a codec Komga does not ship
/// ([`ZipError::MissingCodec`]), or a member read fails [`transient`]ly.
pub(super) fn entries(
    path: &Path,
    analyze_dimensions: bool,
) -> Result<Vec<ContainerEntry>, ZipError> {
    let mut archive = ZipArchive::new(BufReader::new(File::open(path)?))?;
    let mut entries = Vec::new();
    for index in 0..archive.entries().len() {
        let entry = &archive.entries()[index];
        if entry.is_directory() {
            continue;
        }
        let name = entry.name().to_owned();
        let file_size = i64::try_from(entry.size()).ok();
        let (media_type, dimension) = match inspect(&mut archive, index, analyze_dimensions) {
            Ok((media_type, dimension)) => (Some(media_type), dimension),
            Err(error @ ZipError::MissingCodec(_)) => return Err(error),
            Err(error) if transient(&error).is_some() => return Err(error),
            Err(error) => {
                warn!(entry = name, %error, "Could not analyze entry");
                (None, None)
            }
        };
        entries.push(ContainerEntry {
            name,
            media_type,
            dimension,
            file_size,
        });
    }
    entries.sort_by(|left, right| natural_order::compare(&left.name, &right.name));
    Ok(entries)
}

/// Detects the member type from the header Tika reads, then reads the image size from the start
/// of the member again, as Tika's `reset` lets `ImageAnalyzer` do.
fn inspect(
    archive: &mut ZipArchive<BufReader<File>>,
    index: usize,
    analyze_dimensions: bool,
) -> Result<(&'static str, Option<Dimension>), ZipError> {
    let mut reader = archive.entry_reader(index)?;
    let header = media_type::read_header(&mut reader)?;
    let media_type = media_type::detect(header.as_slice())?;
    let dimension = if analyze_dimensions && media_type::is_image(media_type) {
        komga_image::read_dimension(header.as_slice().chain(reader))
    } else {
        None
    };
    Ok((media_type, dimension))
}
