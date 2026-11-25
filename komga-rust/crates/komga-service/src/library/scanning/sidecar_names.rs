//! The sidecar names Kotlin `LocalArtworkProvider` and `MylarSeriesProvider` claim as
//! `SidecarSeriesConsumer` and `SidecarBookConsumer`, also used by the local artwork refresh.

use jiff::civil::DateTime;
use komga_types::{Book, FileUrl, Sidecar, SidecarKind};

use super::ignore_case;

pub(crate) const ARTWORK_EXTENSIONS: [&str; 6] = ["png", "jpeg", "jpg", "tbn", "webp", "gif"];
pub(crate) const SERIES_ARTWORK_NAMES: [&str; 5] =
    ["cover", "default", "folder", "poster", "series"];
const SERIES_JSON: &str = "series.json";

/// `getSidecarSeriesFilenames`, compared ignoring case.
pub(super) fn series_sidecar(file_name: &str) -> Option<SidecarKind> {
    if ignore_case::equals(file_name, SERIES_JSON) {
        return Some(SidecarKind::SeriesMetadata);
    }
    let artwork = SERIES_ARTWORK_NAMES.iter().any(|name| {
        ARTWORK_EXTENSIONS
            .iter()
            .any(|extension| ignore_case::equals(file_name, &format!("{name}.{extension}")))
    });
    artwork.then_some(SidecarKind::SeriesArtwork)
}

/// `getSidecarBookPrefilter`: `.*(-\d+)?\.<extension>` with `IGNORE_CASE`, which reduces to an
/// artwork extension after a stem without Java line terminators, which `.` does not match.
pub(crate) fn is_book_sidecar_candidate(file_name: &str) -> bool {
    let characters: Vec<char> = file_name.chars().collect();
    ARTWORK_EXTENSIONS.iter().any(|extension| {
        let Some(stem_length) = characters.len().checked_sub(extension.len() + 1) else {
            return false;
        };
        let (stem, suffix) = characters.split_at(stem_length);
        suffix[0] == '.'
            && suffix[1..]
                .iter()
                .zip(extension.chars())
                .all(|(&actual, expected)| ignore_case::chars_equal(actual, expected))
            && !stem.iter().copied().any(is_java_line_terminator)
    })
}

/// A file that `is_book_sidecar_candidate` kept until the directory's books are known.
pub(super) struct BookSidecarCandidate {
    pub(super) file_name: String,
    pub(super) url: FileUrl,
    pub(super) last_modified_time: DateTime,
}

/// The candidates matching `books`, in book order; a sidecar belongs to the first matching book.
pub(super) fn match_book_sidecars(
    books: &[Book],
    mut candidates: Vec<BookSidecarCandidate>,
) -> Vec<Sidecar> {
    let mut sidecars = Vec::new();
    for book in books {
        let (matched, rest): (Vec<_>, Vec<_>) = candidates
            .into_iter()
            .partition(|candidate| is_book_sidecar(&book.name, &candidate.file_name));
        candidates = rest;
        sidecars.extend(matched.into_iter().map(|candidate| Sidecar {
            url: candidate.url,
            parent_url: book.url.clone(),
            last_modified_time: candidate.last_modified_time,
            kind: SidecarKind::BookArtwork,
        }));
    }
    sidecars
}

/// `isSidecarBookMatch`: the sidecar base name is `Regex.escape(bookName)(-\d+)?` with
/// `IGNORE_CASE`, where `\d` is ASCII.
pub(crate) fn is_book_sidecar(book_name: &str, file_name: &str) -> bool {
    let mut rest = base_name(file_name).chars();
    for expected in book_name.chars() {
        match rest.next() {
            Some(actual) if ignore_case::chars_equal(actual, expected) => {}
            _ => return false,
        }
    }
    let rest = rest.as_str();
    rest.is_empty()
        || rest.strip_prefix('-').is_some_and(|digits| {
            !digits.is_empty() && digits.bytes().all(|byte| byte.is_ascii_digit())
        })
}

/// Commons IO `FilenameUtils.getBaseName`: after the last `/` or `\`, before the last `.`.
pub(crate) fn base_name(file_name: &str) -> &str {
    let name = file_name.rsplit(['/', '\\']).next().unwrap_or(file_name);
    name.rsplit_once('.').map_or(name, |(base, _)| base)
}

fn is_java_line_terminator(character: char) -> bool {
    matches!(character, '\n' | '\r' | '\u{85}' | '\u{2028}' | '\u{2029}')
}

#[cfg(test)]
mod tests {
    use komga_types::SidecarKind;

    use super::{is_book_sidecar, is_book_sidecar_candidate, series_sidecar};

    #[test]
    fn series_sidecars_match_whole_names_ignoring_case() {
        // `LocalArtworkProviderTest` names; content detection is left to the artwork refresh.
        for name in [
            "CoVeR.jpeg",
            "DefauLt.tbn",
            "POSter.PNG",
            "FoLDer.jpeg",
            "serIES.TBN",
            "serIes.WebP",
            "cover.gif",
        ] {
            assert_eq!(
                series_sidecar(name),
                Some(SidecarKind::SeriesArtwork),
                "{name}"
            );
        }
        assert_eq!(
            series_sidecar("Series.JSON"),
            Some(SidecarKind::SeriesMetadata)
        );
        for name in ["artwork.jpg", "cover.avif", "series.jxl", "my cover.jpg"] {
            assert_eq!(series_sidecar(name), None, "{name}");
        }
    }

    #[test]
    fn book_sidecars_match_the_book_name_and_an_optional_counter() {
        // `LocalArtworkProviderTest` names for the book `book(e).cbz`.
        let book = "book(e)";
        for name in [
            "bOOk(e).jpeg",
            "Book(e).tbn",
            "book(e).PNG",
            "book(e)-1.jpeg",
            "book(e)-23.png",
            "book(e)-123.webp",
            "book(e).gif",
        ] {
            assert!(is_book_sidecar_candidate(name), "{name}");
            assert!(is_book_sidecar(book, name), "{name}");
        }
        for name in [
            "book12(e).jpeg",
            "cover.png",
            "book.webp",
            "book(e)-.png",
            "book(e)-1a.png",
        ] {
            assert!(!is_book_sidecar(book, name), "{name}");
        }
        for name in ["book(e).avif", "book(e).jxl", "line\nbreak.png", "png"] {
            assert!(!is_book_sidecar_candidate(name), "{name}");
        }
        // Java's Unicode case rules: dotless `ı` uppercases to `I`, and digits are ASCII only.
        assert!(is_book_sidecar_candidate("a.G\u{131}F"));
        assert!(is_book_sidecar("Ii", "\u{131}\u{130}.png"));
        assert!(!is_book_sidecar("a", "a-\u{661}.png"));
        // Regex metacharacters in the book name are literal.
        assert!(!is_book_sidecar("a.b", "axb.png"));
    }
}
