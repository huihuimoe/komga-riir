//! Kotlin `EpubExtractor.getDivinaPages`: the images of an EPUB whose every spine item shows one
//! image and almost no text, as pages.

use std::io::Read;

use komga_types::{BookPage, Dimension};
use tracing::info;

use super::xml::{self, Element};
use super::{EpubError, EpubPackage, href};
use crate::media_type;

/// Reads the dimension of an image member, `None` when it cannot.
type ReadDimension = dyn Fn(&mut dyn Read) -> Option<Dimension>;

impl EpubPackage {
    /// The pages of a DiViNa-compatible book, else none. A spine document with more than
    /// `letter_count_threshold` UTF-16 units of body text makes the book incompatible. Spine and
    /// image hrefs are not URL-decoded, as in Kotlin.
    ///
    /// # Errors
    /// A spine document or an image cannot be read, or a typed image has no member.
    pub fn divina_pages(
        &mut self,
        letter_count_threshold: i32,
        read_dimension: Option<&ReadDimension>,
    ) -> Result<Vec<BookPage>, EpubError> {
        let spine: Vec<(String, String)> = self
            .spine()
            .iter()
            .map(|item| {
                let path = href::normalize_href(self.opf_dir(), &item.href);
                (path, item.media_type.clone())
            })
            .collect();
        let page_count = self
            .archive
            .entries()
            .iter()
            .filter(|entry| spine.iter().any(|(path, _)| path == entry.name()))
            .count();

        let Some(pages_with_images) = self.images_by_page(&spine, letter_count_threshold)? else {
            return Ok(Vec::new());
        };
        if pages_with_images.len() != page_count {
            info!(
                pages_with_images = pages_with_images.len(),
                page_count, "Epub Divina detection failed: not every page has images"
            );
            return Ok(Vec::new());
        }
        // KCC sometimes writes the same image several times on a page.
        let mut images: Vec<String> = Vec::new();
        for page in pages_with_images {
            let mut distinct: Vec<String> = Vec::new();
            for image in page {
                if !distinct.contains(&image) {
                    distinct.push(image);
                }
            }
            images.extend(distinct);
        }
        if images.len() != page_count {
            info!(
                images = images.len(),
                page_count, "Epub Divina detection failed: not one image per page"
            );
            return Ok(Vec::new());
        }

        let pages = self.image_pages(images, read_dimension)?;
        if pages.len() != page_count {
            info!(
                pages = pages.len(),
                page_count, "Epub Divina detection failed: not every image is a typed member"
            );
            return Ok(Vec::new());
        }
        Ok(pages)
    }

    /// Kotlin `pagesWithImages`: the images each spine item shows, resolved against its folder;
    /// `None` when a document has too much text.
    fn images_by_page(
        &mut self,
        spine: &[(String, String)],
        letter_count_threshold: i32,
    ) -> Result<Option<Vec<Vec<String>>>, EpubError> {
        let mut pages_with_images: Vec<Vec<String>> = Vec::new();
        for (page_path, page_type) in spine {
            if page_type
                .get(..5)
                .is_some_and(|prefix| prefix.eq_ignore_ascii_case("image"))
            {
                pages_with_images.push(vec![href::normalize(page_path)]);
                continue;
            }
            let Some(content) = self.archive.read_named(page_path)? else {
                pages_with_images.push(Vec::new());
                continue;
            };
            let document = xml::parse(&content);
            let letters = body_text(&document).encode_utf16().count();
            if i64::try_from(letters).unwrap_or(i64::MAX) > i64::from(letter_count_threshold) {
                return Ok(None);
            }
            let page_dir = href::parent(page_path).unwrap_or_default();
            pages_with_images.push(
                image_sources(&document)
                    .into_iter()
                    .map(|source| href::normalize(&href::resolve(&page_dir, &source)))
                    .collect(),
            );
        }
        Ok(Some(pages_with_images))
    }

    /// The images typed as images in the manifest, as pages; others are skipped.
    fn image_pages(
        &mut self,
        images: Vec<String>,
        read_dimension: Option<&ReadDimension>,
    ) -> Result<Vec<BookPage>, EpubError> {
        let mut pages = Vec::new();
        for image in images {
            let Some(media_type) = self
                .manifest
                .iter()
                .find(|item| href::normalize_href(self.opf_dir(), &item.href) == image)
                .map(|item| item.media_type.clone())
            else {
                continue;
            };
            if !media_type::is_image(&media_type) {
                continue;
            }
            let index = self
                .archive
                .entries()
                .iter()
                .position(|entry| entry.name() == image)
                .ok_or_else(|| EpubError::MissingMember(image.clone()))?;
            let file_size = i64::try_from(self.archive.entries()[index].size()).ok();
            let dimension = match read_dimension {
                Some(read_dimension) => read_dimension(&mut self.archive.entry_reader(index)?),
                None => None,
            };
            pages.push(BookPage {
                file_name: image,
                media_type,
                dimension,
                file_hash: String::new(),
                file_size,
            });
        }
        Ok(pages)
    }
}

/// jsoup `Document.body().text()`: the `body` (or `frameset`) child of the top-level `html`.
fn body_text(document: &Element) -> String {
    document
        .children()
        .find(|element| element.is("html"))
        .and_then(|html| {
            html.children()
                .find(|element| element.is("body") || element.is("frameset"))
        })
        .map(Element::text)
        .unwrap_or_default()
}

/// jsoup `getElementsByTag("img")` sources, then `select("svg > image[xlink:href]")` links.
fn image_sources(document: &Element) -> Vec<String> {
    let images = document
        .descendants()
        .into_iter()
        .filter(|(_, element)| element.is("img"))
        .map(|(_, element)| element.attr("src").to_owned());
    let svg_images = document
        .select_child_of(
            |parent| parent.is("svg"),
            |child| {
                child.is("image")
                    && child
                        .attributes()
                        .any(|(key, _)| key.eq_ignore_ascii_case("xlink:href"))
            },
        )
        .into_iter()
        .map(|element| element.attr("xlink:href").to_owned());
    images.chain(svg_images).collect()
}

#[cfg(test)]
mod tests {
    use std::io::Read;

    use komga_test_support::TestDir;
    use komga_types::{BookPage, Dimension};

    use crate::epub::tests::{CONTAINER, write_zip};
    use crate::epub::{EpubError, EpubPackage};

    const OPF: &str = r#"<package><manifest>
        <item id="p1" href="p1.xhtml" media-type="application/xhtml+xml"/>
        <item id="p2" href="text/p2.xhtml" media-type="application/xhtml+xml"/>
        <item id="i3" href="img/3.png" media-type="image/png"/>
        <item id="i1" href="img/1.png" media-type="image/png"/>
        <item id="i2" href="img/2.jpg" media-type="image/jpeg"/>
      </manifest>
      <spine><itemref idref="p1"/><itemref idref="p2"/><itemref idref="i3"/></spine></package>"#;

    /// Two documents showing one image each, the first twice and with five letters, then an image.
    fn book(dir: &TestDir, p1: &str, images: &[&str]) -> EpubPackage {
        let p2 = r#"<html><body><svg><image xlink:href="../img/2.jpg"/></svg></body></html>"#;
        let mut members = vec![
            ("META-INF/container.xml", CONTAINER.as_bytes()),
            ("OPS/book.opf", OPF.as_bytes()),
            ("OPS/p1.xhtml", p1.as_bytes()),
            ("OPS/text/p2.xhtml", p2.as_bytes()),
        ];
        for &image in images {
            members.push((image, image.as_bytes()));
        }
        EpubPackage::open(&write_zip(dir, &members)).unwrap()
    }

    const P1: &str =
        r#"<html><body><p>a b c</p><img src="img/1.png"/><img src="./img/1.png"/></body></html>"#;
    const IMAGES: [&str; 3] = ["OPS/img/1.png", "OPS/img/2.jpg", "OPS/img/3.png"];

    /// The member length as width, to show which member was read.
    fn length(reader: &mut dyn Read) -> Option<Dimension> {
        let mut data = Vec::new();
        reader.read_to_end(&mut data).ok()?;
        Some(Dimension {
            width: u32::try_from(data.len()).ok()?,
            height: 1,
        })
    }

    #[test]
    fn pages_are_the_images_each_spine_item_shows() {
        let dir = TestDir::new().unwrap();
        let page = |name: &str, media_type: &str, dimension| BookPage {
            file_name: name.to_owned(),
            media_type: media_type.to_owned(),
            dimension,
            file_hash: String::new(),
            file_size: Some(i64::try_from(name.len()).unwrap()),
        };
        let width = |name: &str| {
            Some(Dimension {
                width: u32::try_from(name.len()).unwrap(),
                height: 1,
            })
        };
        let mut epub = book(&dir, P1, &IMAGES);
        assert_eq!(
            epub.divina_pages(5, Some(&length)).unwrap(),
            vec![
                page(IMAGES[0], "image/png", width(IMAGES[0])),
                page(IMAGES[1], "image/jpeg", width(IMAGES[1])),
                page(IMAGES[2], "image/png", width(IMAGES[2])),
            ],
            "a repeated image counts once, and five letters are within the threshold"
        );
        assert!(
            epub.divina_pages(5, None).unwrap()[0].dimension.is_none(),
            "dimensions are read on request"
        );
        assert_eq!(
            epub.divina_pages(4, None).unwrap(),
            Vec::new(),
            "more letters than the threshold"
        );
    }

    #[test]
    fn incomplete_page_mappings_are_not_divina() {
        let dir = TestDir::new().unwrap();
        let two_images =
            r#"<html><body><img src="img/1.png"/><img src="img/2.jpg"/></body></html>"#;
        assert_eq!(
            book(&dir, two_images, &IMAGES)
                .divina_pages(15, None)
                .unwrap(),
            Vec::new(),
            "four images for three pages"
        );
        let untyped = r#"<html><body><img src="img/9.png"/></body></html>"#;
        assert_eq!(
            book(&dir, untyped, &IMAGES).divina_pages(15, None).unwrap(),
            Vec::new(),
            "an image outside the manifest"
        );
        assert!(
            matches!(
                book(&dir, P1, &IMAGES[1..]).divina_pages(15, None),
                Err(EpubError::MissingMember(name)) if name == IMAGES[0]
            ),
            "Kotlin fails on the null entry of a typed image without member"
        );
    }
}
