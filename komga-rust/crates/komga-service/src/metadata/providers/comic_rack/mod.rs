//! Kotlin `ComicInfoProvider`: metadata from the `ComicInfo.xml` file at the root of a book.

use jiff::civil::Date;
use komga_archive::divina::DivinaFormat;
use komga_archive::zip;
use komga_types::{
    Book, BookAuthor, Media, MediaProfile, MediaStatus, NotableEntity, ReadingDirection, WebLink,
    normalize_language_tag, validate_isbn,
};
use tokio::task;
use tracing::{debug, error};
use url::Url;

use super::super::edit::patch::{BookMetadataPatch, ReadListEntry};
use super::super::edit::series_patch::SeriesMetadataPatch;
use super::comic_info::{self, ComicInfo};

const COMIC_INFO: &str = "ComicInfo.xml";

/// `getBookMetadataFromBook`; `None` without a readable `ComicInfo.xml`.
pub(super) async fn book_patch(book: &Book, media: &Media) -> Option<BookMetadataPatch> {
    let comic_info = read(book, media).await?;
    let patch = patch(&comic_info);
    if patch.is_none() {
        error!(book = %book.id, "Error while getting metadata from ComicInfoProvider: invalid date");
    }
    patch
}

/// `getSeriesMetadataFromBook`; `None` without a readable `ComicInfo.xml`.
pub(super) async fn series_patch(
    book: &Book,
    media: &Media,
    append_volume: bool,
) -> Option<SeriesMetadataPatch> {
    let comic_info = read(book, media).await?;
    Some(series(&comic_info, append_volume))
}

/// The series patch of `comic_info`. A blank `Series` is no title, also without
/// `append_volume` where Kotlin would take it.
fn series(comic_info: &ComicInfo, append_volume: bool) -> SeriesMetadataPatch {
    let title = not_blank(comic_info.series.as_ref()).map(|series| match comic_info.volume {
        // `computeSeriesFromSeriesAndVolume`.
        Some(volume) if append_volume && volume != 1 => format!("{series} ({volume})"),
        _ => series,
    });
    let genres: Option<Vec<String>> = comic_info
        .genre
        .as_deref()
        .map(|genres| split(genres).map(str::to_owned).collect());
    SeriesMetadataPatch {
        title_sort: title.clone(),
        title,
        status: None,
        summary: None,
        // Kotlin `Manga`, whose values match exactly.
        reading_direction: match comic_info.manga.as_deref() {
            Some("No") => Some(ReadingDirection::LeftToRight),
            Some("YesAndRightToLeft") => Some(ReadingDirection::RightToLeft),
            _ => None,
        },
        publisher: not_blank(comic_info.publisher.as_ref()),
        age_rating: comic_info.age_rating.as_deref().and_then(age_rating),
        language: comic_info
            .language_iso
            .as_deref()
            .and_then(normalize_language_tag),
        genres: genres.filter(|genres| !genres.is_empty()),
        total_book_count: comic_info.count,
        collections: comic_info
            .series_group
            .as_deref()
            .into_iter()
            .flat_map(split)
            .map(str::to_owned)
            .collect(),
    }
}

/// Kotlin `AgeRating.fromValue`, matched lowercased without spaces.
fn age_rating(value: &str) -> Option<i32> {
    match value.to_lowercase().replace(' ', "").as_str() {
        "everyone" | "g" => Some(0),
        "earlychildhood" => Some(3),
        "kidstoadults" => Some(6),
        "pg" => Some(8),
        "everyone10+" => Some(10),
        "teen" => Some(13),
        "ma15+" => Some(15),
        "m" | "mature17+" => Some(17),
        "adultsonly18+" | "r18+" | "x18+" => Some(18),
        _ => None,
    }
}

/// `getComicInfo`: errors are logged.
async fn read(book: &Book, media: &Media) -> Option<ComicInfo> {
    if !media.files.iter().any(|file| file.file_name == COMIC_INFO) {
        debug!(book = %book.id, "Book does not contain any {COMIC_INFO} file");
        return None;
    }
    // `BookAnalyzer.getFileContent`.
    if media.status != MediaStatus::Ready {
        error!(book = %book.id, "Book media is not ready, cannot get files");
        return None;
    }
    let media_type = media.media_type.clone().unwrap_or_default();
    let Some(path) = book.url.to_path() else {
        error!(book = %book.id, url = %book.url, "Book URL is not a local file");
        return None;
    };
    let read = task::spawn_blocking(move || {
        let content = match MediaProfile::from_media_type(Some(&media_type)) {
            Some(MediaProfile::Divina) => DivinaFormat::from_media_type(&media_type)
                .expect("Divina media types have a format")
                .read_entry(&path, COMIC_INFO)
                .map_err(|error| error.to_string())?,
            Some(MediaProfile::Epub) => {
                zip::read_entry(&path, COMIC_INFO).map_err(|error| error.to_string())?
            }
            Some(MediaProfile::Pdf) | None => {
                return Err("Extractor does not support extraction of files".to_owned());
            }
        };
        comic_info::parse(&content).map_err(|error| error.to_string())
    })
    .await
    .unwrap_or_else(|error| Err(error.to_string()));
    read.inspect_err(|message| {
        error!(book = %book.id, "Error while retrieving metadata from {COMIC_INFO}: {message}");
    })
    .ok()
}

/// The book patch of `comic_info`; `None` when its date does not exist, which fails the whole
/// provider in Kotlin.
fn patch(comic_info: &ComicInfo) -> Option<BookMetadataPatch> {
    let release_date = match comic_info.year {
        Some(year) => Some(date(year, comic_info.month, comic_info.day)?),
        None => None,
    };
    let c = comic_info;
    let roles = [
        (&c.writer, "writer"),
        (&c.penciller, "penciller"),
        (&c.inker, "inker"),
        (&c.colorist, "colorist"),
        (&c.letterer, "letterer"),
        (&c.cover_artist, "cover"),
        (&c.editor, "editor"),
        (&c.translator, "translator"),
    ];
    let authors: Vec<BookAuthor> = roles
        .into_iter()
        .flat_map(|(names, role)| {
            let names = names.as_deref().into_iter().flat_map(split);
            names.map(move |name| BookAuthor::new(name, role))
        })
        .collect();
    let kinds = [
        (&c.locations, "location"),
        (&c.teams, "team"),
        (&c.characters, "character"),
    ];
    let notable_entities: Vec<NotableEntity> = kinds
        .into_iter()
        .flat_map(|(names, kind)| {
            let names = names.as_deref().into_iter().flat_map(split);
            names.map(move |name| NotableEntity::new(kind, name))
        })
        .collect();
    let links: Option<Vec<WebLink>> = comic_info.web.as_ref().map(|web| {
        web.split(' ')
            .filter(|link| !link.trim().is_empty())
            .filter_map(link)
            .collect()
    });
    let tags: Option<Vec<String>> = comic_info
        .tags
        .as_ref()
        .map(|tags| split(tags).map(str::to_lowercase).collect());
    Some(BookMetadataPatch {
        title: not_blank(comic_info.title.as_ref()),
        summary: not_blank(comic_info.summary.as_ref()),
        number: not_blank(comic_info.number.as_ref()),
        number_sort: comic_info.number.as_deref().and_then(parse_float),
        release_date,
        authors: (!authors.is_empty()).then_some(authors),
        notable_entities: (!notable_entities.is_empty()).then_some(notable_entities),
        isbn: comic_info.gtin.as_deref().and_then(validate_isbn),
        links: links.filter(|links| !links.is_empty()),
        tags: tags.filter(|tags| !tags.is_empty()),
        read_lists: read_lists(comic_info),
    })
}

/// `LocalDate.of(year, month ?: 1, day ?: 1)`.
fn date(year: i32, month: Option<i32>, day: Option<i32>) -> Option<Date> {
    Date::new(
        i16::try_from(year).ok()?,
        i8::try_from(month.unwrap_or(1)).ok()?,
        i8::try_from(day.unwrap_or(1)).ok()?,
    )
    .ok()
}

/// `AlternateSeries` with `AlternateNumber`, then each `StoryArc` with the `StoryArcNumber` at
/// the same position; without numbers, each arc unnumbered.
fn read_lists(comic_info: &ComicInfo) -> Vec<ReadListEntry> {
    let mut entries = Vec::new();
    if let Some(name) = not_blank(comic_info.alternate_series.as_ref()) {
        entries.push(ReadListEntry {
            name,
            number: comic_info
                .alternate_number
                .as_deref()
                .and_then(|number| number.parse().ok()),
        });
    }
    if let Some(arcs) = &comic_info.story_arc {
        let arcs: Vec<Option<&str>> = arcs
            .split(',')
            .map(|arc| Some(arc.trim()).filter(|arc| !arc.is_empty()))
            .collect();
        let numbers: Vec<Option<i32>> = comic_info
            .story_arc_number
            .as_deref()
            .map(|numbers| numbers.split(',').map(|n| n.trim().parse().ok()).collect())
            .unwrap_or_default();
        if numbers.is_empty() {
            entries.extend(arcs.into_iter().flatten().map(|name| ReadListEntry {
                name: name.to_owned(),
                number: None,
            }));
        } else {
            for (arc, number) in arcs.into_iter().zip(numbers) {
                if let (Some(name), Some(number)) = (arc, number) {
                    entries.push(ReadListEntry {
                        name: name.to_owned(),
                        number: Some(number),
                    });
                }
            }
        }
    }
    entries
}

/// `URI(link.trim())` labeled by its host; links without a host are skipped.
fn link(link: &str) -> Option<WebLink> {
    let url = link.trim();
    match Url::parse(url) {
        Ok(parsed) => Some(WebLink {
            label: parsed.host_str()?.to_owned(),
            url: url.to_owned(),
        }),
        Err(error) => {
            error!("Could not parse Web element as valid URI: {url}: {error}");
            None
        }
    }
}

/// The trimmed, non-blank items of a comma-separated list.
fn split(list: &str) -> impl Iterator<Item = &str> {
    list.split(',')
        .map(str::trim)
        .filter(|item| !item.is_empty())
}

fn not_blank(value: Option<&String>) -> Option<String> {
    value.filter(|value| !value.trim().is_empty()).cloned()
}

/// Kotlin `String.toFloatOrNull` for finite decimal numbers: surrounding control characters and
/// spaces are ignored. Infinities and NaN, which would break sorting, are not numbers here.
pub(super) fn parse_float(value: &str) -> Option<f32> {
    let number: f32 = value.trim_matches(|c: char| c <= ' ').parse().ok()?;
    number.is_finite().then_some(number)
}

#[cfg(test)]
mod tests;
