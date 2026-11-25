//! Kotlin `EpubExtractor.isKepub` and `computePositions`: the Readium positions of an EPUB, with
//! the `koboSpan` nearest to each.

use std::collections::HashMap;

use komga_types::{MediaFileSubType, R2Location, R2Locator};
use tracing::warn;

use super::kepub::{self, KoboSpan};
use super::{EpubPackage, EpubResource};
use crate::zip::ZipError;

/// The span of the first position of every resource, which Kobo devices accept without lookup.
const FIRST_KOBO_SPAN: &str = "kobo.1.1";

impl EpubPackage {
    /// Kotlin `isKepub`: a reading order document has a `koboSpan` element. Unreadable documents
    /// are skipped, and a read failure makes the book no KEPUB.
    #[must_use]
    pub fn is_kepub(&mut self, resources: &[EpubResource]) -> bool {
        for resource in reading_order(resources) {
            match self.archive.read_named(&resource.file_name) {
                Ok(Some(content)) => {
                    if kepub::has_kobo_span(&String::from_utf8_lossy(&content)) {
                        return true;
                    }
                }
                Ok(None) => {}
                Err(error) => {
                    warn!(%error, "Error while checking if EPUB is KEPUB");
                    return false;
                }
            }
        }
        false
    }

    /// Kotlin `computePositions` over `resources` that all have a size: one position per page of
    /// a fixed layout, else one per started KiB of each reading order document, then the total
    /// progression of each.
    ///
    /// The spans of a book that is no KEPUB come from the built-in kepubify transform of its
    /// content documents, which stands in for Kotlin's optional external kepubify; like a failed
    /// conversion, a document that cannot be read leaves every position without span.
    ///
    /// # Errors
    /// A reading order document of a KEPUB cannot be read.
    pub fn positions(
        &mut self,
        resources: &[EpubResource],
        is_fixed_layout: bool,
        is_kepub: bool,
    ) -> Result<Vec<R2Locator>, ZipError> {
        let reading_order: Vec<&EpubResource> = reading_order(resources).collect();
        let kobo_positions = if is_fixed_layout {
            HashMap::new()
        } else if is_kepub {
            self.kobo_positions(&reading_order, |content, _| {
                String::from_utf8_lossy(content).into_owned()
            })?
        } else {
            self.kobo_positions(&reading_order, |content, resource| {
                if is_content_document(resource) {
                    kepub::transform_content(content)
                } else {
                    String::from_utf8_lossy(content).into_owned()
                }
            })
            .unwrap_or_else(|error| {
                warn!(%error, "Could not convert to Kepub to compute positions");
                HashMap::new()
            })
        };

        Ok(build_positions(
            &reading_order,
            is_fixed_layout,
            &kobo_positions,
        ))
    }

    /// Kotlin `computePositionsFromKoboSpan`: per document, its spans with the end of their start
    /// tag over the document size. A missing document has no entry.
    fn kobo_positions(
        &mut self,
        reading_order: &[&EpubResource],
        html: impl Fn(&[u8], &EpubResource) -> String,
    ) -> Result<HashMap<String, Vec<(String, f32)>>, ZipError> {
        let mut positions = HashMap::new();
        for &resource in reading_order {
            let Some(content) = self.archive.read_named(&resource.file_name)? else {
                continue;
            };
            let size = resource
                .file_size
                .expect("Positions are computed for found resources");
            positions.insert(
                resource.file_name.clone(),
                spans_for_document(&html(&content, resource), size),
            );
        }
        Ok(positions)
    }
}

/// Computes `koboSpan` progressions over a single transformed XHTML document.
pub(crate) fn spans_for_document(transformed_html: &str, document_size: u64) -> Vec<(String, f32)> {
    kepub::kobo_spans(transformed_html)
        .into_iter()
        .map(|KoboSpan { id, end }| (id, ratio(end as u64, document_size.max(1))))
        .collect()
}

/// Builds the Readium position list for `reading_order` using precomputed `kobo_positions`.
pub(crate) fn build_positions(
    reading_order: &[&EpubResource],
    is_fixed_layout: bool,
    kobo_positions: &HashMap<String, Vec<(String, f32)>>,
) -> Vec<R2Locator> {
    let mut steps: Vec<(&EpubResource, f32, Option<String>)> = Vec::new();
    for &resource in reading_order {
        if is_fixed_layout {
            steps.push((resource, 0.0, Some(FIRST_KOBO_SPAN.to_owned())));
            continue;
        }
        let size = resource
            .file_size
            .expect("Positions are computed for found resources");
        let count = size.div_ceil(1024).max(1);
        for index in 0..count {
            let progression = ratio(index, count);
            let kobo_span = if index == 0 {
                Some(FIRST_KOBO_SPAN.to_owned())
            } else {
                nearest_span(kobo_positions.get(&resource.file_name), progression)
            };
            steps.push((resource, progression, kobo_span));
        }
    }

    let total = steps.len() as u64;
    (1_u64..)
        .zip(steps)
        .map(|(position, (resource, progression, kobo_span))| R2Locator {
            href: resource.file_name.clone(),
            media_type: resource.media_type.clone(),
            title: None,
            locations: Some(R2Location {
                fragments: Vec::new(),
                progression: Some(progression),
                position: Some(i32::try_from(position).expect("Fewer than 2^31 positions")),
                total_progression: Some(ratio(position, total)),
            }),
            text: None,
            kobo_span,
        })
        .collect()
}

fn reading_order(resources: &[EpubResource]) -> impl Iterator<Item = &EpubResource> {
    resources
        .iter()
        .filter(|resource| resource.sub_type == MediaFileSubType::EpubPage)
}

/// kepubify `epubContentDocuments`: an (X)HTML media type or file extension.
fn is_content_document(resource: &EpubResource) -> bool {
    matches!(
        resource.media_type.as_str(),
        "application/xhtml+xml" | "text/html"
    ) || resource
        .file_name
        .rsplit_once('.')
        .is_some_and(|(_, extension)| {
            ["htm", "html", "xhtml"].contains(&extension.to_ascii_lowercase().as_str())
        })
}

/// Kotlin `minByOrNull { (progression - it.second).absoluteValue }`: the first nearest span.
fn nearest_span(spans: Option<&Vec<(String, f32)>>, progression: f32) -> Option<String> {
    spans?
        .iter()
        .min_by(|(_, left), (_, right)| {
            (progression - left)
                .abs()
                .total_cmp(&(progression - right).abs())
        })
        .map(|(id, _)| id.clone())
}

/// Kotlin `a.toFloat() / b.toFloat()`.
fn ratio(numerator: u64, denominator: u64) -> f32 {
    numerator as f32 / denominator as f32
}

#[cfg(test)]
mod tests {
    use std::path::{Path, PathBuf};

    use komga_test_support::TestDir;
    use komga_types::{R2Location, R2Locator};

    use crate::epub::tests::{CONTAINER, write_zip};
    use crate::epub::{EpubPackage, EpubResource};

    const OPF: &str = r#"<package><manifest>
        <item id="a" href="a.xhtml" media-type="application/xhtml+xml"/>
        <item id="b" href="b.xhtml" media-type="application/xhtml+xml"/>
        <item id="c" href="c.png" media-type="image/png"/>
      </manifest>
      <spine><itemref idref="a"/><itemref idref="b"/><itemref idref="c"/></spine></package>"#;

    /// A 2500-byte document, so three positions, with start tags of spans ending at 2 %, 44 % and
    /// 84 %.
    fn kepub_document() -> String {
        let start_tag = |id: &str| format!(r#"<span class="koboSpan" id="{id}">"#);
        let end = "</p></body></html>\n";
        let mut html = format!("<html><body><p>{}x</span>", start_tag("kobo.1.1"));
        for (id, at) in [("kobo.2.1", 1100), ("kobo.3.1", 2100)] {
            let tag = start_tag(id);
            html.push_str(&" ".repeat(at - html.len() - tag.len()));
            html.push_str(&tag);
            html.push_str("x</span>");
        }
        html.push_str(&" ".repeat(2500 - html.len() - end.len()));
        html.push_str(end);
        html
    }

    fn book(dir: &TestDir, a: &str, b: &str) -> (EpubPackage, Vec<EpubResource>) {
        let path = write_zip(
            dir,
            &[
                ("META-INF/container.xml", CONTAINER),
                ("OPS/book.opf", OPF),
                ("OPS/a.xhtml", a),
                ("OPS/b.xhtml", b),
                ("OPS/c.png", "png"),
            ],
        );
        let epub = EpubPackage::open(&path).unwrap();
        let resources = epub.resources().unwrap();
        (epub, resources)
    }

    fn spans(positions: &[R2Locator]) -> Vec<Option<&str>> {
        positions
            .iter()
            .map(|locator| locator.kobo_span.as_deref())
            .collect()
    }

    #[test]
    fn kepub_positions_take_the_nearest_span_of_the_document() {
        let dir = TestDir::new().unwrap();
        let (mut epub, resources) = book(&dir, &kepub_document(), "<p>b</p>");
        assert!(epub.is_kepub(&resources));
        let positions = epub.positions(&resources, false, true).unwrap();
        assert_eq!(
            spans(&positions),
            [
                Some("kobo.1.1"),
                Some("kobo.2.1"),
                Some("kobo.3.1"),
                Some("kobo.1.1"),
                Some("kobo.1.1"),
            ],
            "1/3 is nearest 44 %, 2/3 nearest 84 %; the first position of a document is kobo.1.1"
        );
        let location = |progression, position, total_progression| {
            Some(R2Location {
                fragments: Vec::new(),
                progression: Some(progression),
                position: Some(position),
                total_progression: Some(total_progression),
            })
        };
        assert_eq!(
            positions[1],
            R2Locator {
                href: "OPS/a.xhtml".to_owned(),
                media_type: "application/xhtml+xml".to_owned(),
                title: None,
                locations: location(1.0 / 3.0, 2, 2.0 / 5.0),
                text: None,
                kobo_span: Some("kobo.2.1".to_owned()),
            }
        );
        assert_eq!(
            positions[4].href, "OPS/c.png",
            "spine images have positions too"
        );
        assert_eq!(positions[4].locations, location(0.0, 5, 1.0));
    }

    #[test]
    fn other_books_take_spans_from_the_kepub_transform() {
        let dir = TestDir::new().unwrap();
        let sentences = "Sentence. ".repeat(250);
        let a = format!("<html><body><p>{sentences}</p></body></html>");
        let (mut epub, resources) = book(&dir, &a, "<p>b</p>");
        assert!(!epub.is_kepub(&resources));
        let positions = epub.positions(&resources, false, false).unwrap();
        assert_eq!(positions.len(), 5, "{a} is 2541 bytes");
        assert!(
            spans(&positions[1..3])
                .iter()
                .all(|span| span
                    .is_some_and(|span| span.starts_with("kobo.1.") && span != "kobo.1.1")),
            "the transform splits the paragraph into sentences: {:?}",
            spans(&positions)
        );
    }

    #[test]
    fn fixed_layouts_have_one_position_per_document() {
        let dir = TestDir::new().unwrap();
        let (mut epub, resources) = book(&dir, &kepub_document(), "<p>b</p>");
        let positions = epub.positions(&resources, true, true).unwrap();
        assert_eq!(spans(&positions), [Some("kobo.1.1"); 3]);
        let numbers: Vec<_> = positions
            .iter()
            .map(|locator| locator.locations.clone().unwrap())
            .map(|location| {
                (
                    location.progression,
                    location.position,
                    location.total_progression,
                )
            })
            .collect();
        assert_eq!(
            numbers,
            [
                (Some(0.0), Some(1), Some(1.0 / 3.0)),
                (Some(0.0), Some(2), Some(2.0 / 3.0)),
                (Some(0.0), Some(3), Some(1.0)),
            ]
        );
    }

    fn fixture(name: &str) -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .join("tests/fixtures/epub")
            .join(name)
    }

    #[test]
    fn sample_book_has_a_position_per_started_kib() {
        let mut epub =
            EpubPackage::open(&fixture("The Incomplete Theft - Ralph Burke.epub")).unwrap();
        let resources = epub.resources().unwrap();
        assert!(!epub.is_kepub(&resources));
        let positions = epub.positions(&resources, false, false).unwrap();
        // Documents of 818, 1869, 911, and 31228 bytes.
        assert_eq!(positions.len(), 1 + 2 + 1 + 31);
        assert!(
            positions.iter().all(|locator| locator.kobo_span.is_some()),
            "every document has transformed spans"
        );
    }
}
