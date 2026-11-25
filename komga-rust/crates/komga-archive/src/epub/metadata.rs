//! The package document values Kotlin `EpubMetadataProvider` reads, before Komga gives them
//! meaning: the children of `*|metadata` and the first `*|spine`.

use html5gum::{DefaultEmitter, Token, Tokenizer};

use super::EpubPackage;
use super::xml::Element;

/// Text values are trimmed and never blank; absent ones are `None` or empty.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct EpubMetadata {
    pub title: Option<String>,
    /// The text of the first description, HTML markup removed.
    pub description: Option<String>,
    pub date: Option<String>,
    pub creators: Vec<EpubCreator>,
    pub identifiers: Vec<String>,
    /// The first `belongs-to-collection` meta.
    pub collection: Option<String>,
    /// The `group-position` meta refining that collection.
    pub collection_position: Option<String>,
    pub publisher: Option<String>,
    pub language: Option<String>,
    /// Distinct, in document order.
    pub subjects: Vec<String>,
    /// The spine's `page-progression-direction`, as written.
    pub page_progression_direction: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubCreator {
    pub name: String,
    /// `opf:role` as written, else the text of the last MARC relator `role` meta refining the
    /// creator's id.
    pub role: Option<String>,
}

impl EpubPackage {
    #[must_use]
    pub fn metadata(&self) -> EpubMetadata {
        let elements: Vec<&Element> = self
            .opf
            .select_child_of(|parent| parent.is_any_ns("metadata"), |_| true);
        let all = |tag: &'static str| {
            elements
                .iter()
                .copied()
                .filter(move |element| element.is_any_ns(tag))
        };
        let first = |tag: &'static str| all(tag).next().and_then(text);
        let metas: Vec<&Element> = all("meta").collect();
        let collection = metas
            .iter()
            .find(|meta| attr_is(meta, "property", "belongs-to-collection"));
        let collection_position = collection.and_then(|collection| {
            let refines = format!("#{}", collection.attr("id"));
            metas
                .iter()
                .find(|meta| {
                    attr_is(meta, "refines", &refines)
                        && attr_is(meta, "property", "group-position")
                })
                .and_then(|meta| text(meta))
        });
        let mut subjects: Vec<String> = Vec::new();
        for subject in all("subject").filter_map(text) {
            if !subjects.contains(&subject) {
                subjects.push(subject);
            }
        }
        EpubMetadata {
            title: first("title"),
            description: all("description")
                .next()
                .map(|element| html_text(&element.text()))
                .filter(|text| !text.is_empty()),
            date: first("date"),
            creators: creators(&metas, all("creator")),
            identifiers: all("identifier").filter_map(text).collect(),
            collection: collection.and_then(|collection| text(collection)),
            collection_position,
            publisher: first("publisher"),
            language: first("language"),
            subjects,
            page_progression_direction: self
                .opf
                .descendants()
                .into_iter()
                .find(|(_, element)| element.is_any_ns("spine"))
                .map(|(_, spine)| spine.attr("page-progression-direction").to_owned()),
        }
    }
}

fn creators<'tree>(
    metas: &[&Element],
    creators: impl Iterator<Item = &'tree Element>,
) -> Vec<EpubCreator> {
    let relators: Vec<(&str, String)> = metas
        .iter()
        .filter(|meta| {
            attr_is(meta, "property", "role") && attr_is(meta, "scheme", "marc:relators")
        })
        .map(|meta| {
            let refines = meta.attr("refines");
            (refines.strip_prefix('#').unwrap_or(refines), meta.text())
        })
        .collect();
    creators
        .filter_map(|creator| {
            let name = text(creator)?;
            let refined = || {
                let id = Some(creator.attr("id")).filter(|id| !is_blank(id))?;
                let (_, role) = relators.iter().rev().find(|(refines, _)| refines == &id)?;
                Some(role.clone()).filter(|role| !is_blank(role))
            };
            let role = Some(creator.attr("opf:role"))
                .filter(|role| !is_blank(role))
                .map(str::to_owned)
                .or_else(refined);
            Some(EpubCreator { name, role })
        })
        .collect()
}

/// jsoup `[key=value]`: trimmed and ignoring case.
fn attr_is(element: &Element, key: &str, value: &str) -> bool {
    element.attr(key).trim().eq_ignore_ascii_case(value.trim())
}

fn text(element: &Element) -> Option<String> {
    let text = element.text();
    let text = text.trim();
    (!text.is_empty()).then(|| text.to_owned())
}

fn is_blank(value: &str) -> bool {
    value.trim().is_empty()
}

/// The text of `html` as jsoup `Jsoup.clean(html, Safelist.none())` keeps it, without escaping it
/// again: character data outside `script` and `style`, whitespace collapsed.
fn html_text(html: &str) -> String {
    let mut text = String::new();
    let mut in_data = false;
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for token in Tokenizer::new_with_emitter(html, emitter).flatten() {
        match token {
            Token::StartTag(tag) => in_data = matches!(&tag.name[..], b"script" | b"style"),
            Token::EndTag(_) => in_data = false,
            Token::String(string) if !in_data => {
                text.push_str(&String::from_utf8_lossy(&string.value));
            }
            _ => {}
        }
    }
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

#[cfg(test)]
mod tests {
    //! Package documents of Kotlin `EpubMetadataProviderTest`.

    use std::fs;
    use std::path::Path;

    use komga_test_support::TestDir;

    use super::{EpubCreator, EpubMetadata, html_text};
    use crate::epub::EpubPackage;
    use crate::epub::tests::{CONTAINER, write_zip};

    fn metadata(opf: &str) -> EpubMetadata {
        let fixtures = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/epub");
        let opf = fs::read(fixtures.join(opf)).unwrap();
        let dir = TestDir::new().unwrap();
        let path = write_zip(
            &dir,
            &[
                ("META-INF/container.xml", CONTAINER.as_bytes()),
                ("OPS/book.opf", opf.as_slice()),
            ],
        );
        EpubPackage::open(&path).unwrap().metadata()
    }

    fn creator(name: &str, role: &str) -> EpubCreator {
        EpubCreator {
            name: name.to_owned(),
            role: Some(role.to_owned()),
        }
    }

    #[test]
    fn prefixed_epub3_metadata_is_read_with_refinements() {
        let metadata = metadata("Panik im Paradies - namespace.opf");
        assert_eq!(metadata.title.as_deref(), Some("Panik im Paradies"));
        assert!(
            metadata.description.as_deref().is_some_and(|text| text
                .starts_with("Bereits im ersten Band \"Panik im Paradies\"")
                && text.ends_with("hütet...")),
            "the markup is removed: {:?}",
            metadata.description
        );
        assert_eq!(
            metadata.creators,
            [creator("Ulf Blanck", "aut"), creator("The Editor", "edt")]
        );
        assert!(
            metadata
                .identifiers
                .contains(&"isbn:9783440077894".to_owned())
        );
        assert_eq!(metadata.collection.as_deref(), Some("Die drei ??? Kids"));
        assert_eq!(metadata.collection_position.as_deref(), Some("1.5"));
        assert_eq!(metadata.publisher.as_deref(), Some("Kosmos"));
        assert_eq!(metadata.language.as_deref(), Some("de"));
        assert_eq!(metadata.subjects, ["Kinder- und Jugendbücher"]);
        assert_eq!(metadata.page_progression_direction.as_deref(), Some("rtl"));
        assert_eq!(metadata, self::metadata("Panik im Paradies.opf"));
    }

    #[test]
    fn epub2_roles_are_opf_attributes() {
        let metadata = metadata("1979.opf");
        assert_eq!(
            metadata.creators,
            [
                creator("Kracht, Christian", "aut"),
                creator("The Editor", "edt")
            ]
        );
        assert_eq!(metadata.date.as_deref(), Some("0101-01-01T00:00:00+00:00"));
        assert_eq!(metadata.collection, None);
        assert!(
            metadata
                .description
                .as_deref()
                .is_some_and(|text| text.contains("Ich schlief.Später hielten wir an")),
            "elements join their text without separators, as jsoup's cleaner does: {:?}",
            metadata.description
        );
    }

    #[test]
    fn html_text_keeps_character_data_only() {
        assert_eq!(
            html_text(
                "<p>Tom &amp; <b>Jerry</b></p>\n<script>x < y</script><style>p{}</style> 1 < 2 "
            ),
            "Tom & Jerry 1 < 2"
        );
    }
}
