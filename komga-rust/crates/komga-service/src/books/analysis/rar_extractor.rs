//! Kotlin `RarExtractor.getEntries`: the members of a RAR book file, typed from their content.

use std::path::Path;

use komga_archive::media_type;
use komga_archive::rar::{RarArchive, RarError};
use tracing::warn;

use super::ContainerEntry;
use super::failure::transient;
use crate::support::natural_order;

/// The file members in natural name order. A member that cannot be read gets no type; image
/// sizes are read when `analyze_dimensions` is set.
///
/// # Errors
/// The archive cannot be opened, or a member read fails [`transient`]ly.
pub(super) fn entries(
    path: &Path,
    analyze_dimensions: bool,
) -> Result<Vec<ContainerEntry>, RarError> {
    let archive = RarArchive::open(path)?;
    // By archive index; members not visited keep no type.
    let mut entries: Vec<Option<ContainerEntry>> = archive
        .entries()
        .iter()
        .map(|entry| {
            (!entry.is_directory()).then(|| ContainerEntry {
                name: entry.name().to_owned(),
                media_type: None,
                dimension: None,
                file_size: i64::try_from(entry.size()).ok(),
            })
        })
        .collect();
    let mut interrupted = None;
    let read = archive.read_all(|index, data| {
        let entry = entries[index]
            .as_mut()
            .expect("Only file members are visited");
        let inspected = data.and_then(|data| {
            let media_type = media_type::detect(data.as_slice())?;
            let dimension = if analyze_dimensions && media_type::is_image(media_type) {
                komga_image::read_dimension(data.as_slice())
            } else {
                None
            };
            Ok((media_type, dimension))
        });
        match inspected {
            Ok((media_type, dimension)) => {
                entry.media_type = Some(media_type);
                entry.dimension = dimension;
            }
            Err(error) if transient(&error).is_some() => {
                interrupted.get_or_insert(error);
            }
            Err(error) => warn!(entry = entry.name, %error, "Could not analyze entry"),
        }
    });
    if let Some(error) = interrupted {
        return Err(error);
    }
    if let Err(error) = read {
        if transient(&error).is_some() {
            return Err(error);
        }
        warn!(path = %path.display(), %error, "Could not analyze the remaining entries");
    }
    let mut entries: Vec<ContainerEntry> = entries.into_iter().flatten().collect();
    entries.sort_by(|left, right| natural_order::compare(&left.name, &right.name));
    Ok(entries)
}
