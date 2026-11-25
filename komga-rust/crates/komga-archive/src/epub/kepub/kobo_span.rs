//! The `koboSpan` elements jsoup's HTML parser finds in a content document, and where their start
//! tags end in jsoup's `sourceRange()`.
//!
//! Start tags are read with a WHATWG tokenizer that enters the raw text states after `script`,
//! `style`, and the like, as the tree builder would; elements are not built.

use html5gum::{DefaultEmitter, Token, Tokenizer};

/// A `span.koboSpan` with a non-blank id, and the end of its start tag in UTF-16 units.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct KoboSpan {
    pub id: String,
    pub end: usize,
}

/// jsoup `getElementsByClass("koboSpan")` is not empty.
pub(crate) fn has_kobo_span(html: &str) -> bool {
    start_tags(html).any(|(_, attributes, _)| is_kobo_span(&attributes))
}

/// jsoup `select("span.koboSpan")` with a non-blank `id()`, in document order.
pub(crate) fn kobo_spans(html: &str) -> Vec<KoboSpan> {
    let mut spans = Vec::new();
    let mut utf16 = Utf16Offsets::new(html);
    for (name, attributes, end) in start_tags(html) {
        if name != "span" || !is_kobo_span(&attributes) {
            continue;
        }
        let id = attribute(&attributes, "id").unwrap_or_default();
        if !id.trim().is_empty() {
            spans.push(KoboSpan {
                id,
                end: utf16.at(end),
            });
        }
    }
    spans
}

type Attributes = Vec<(String, String)>;

/// Start tags as `(name, attributes, end byte offset)`; names are lowercased by the tokenizer.
fn start_tags(html: &str) -> impl Iterator<Item = (String, Attributes, usize)> + '_ {
    let mut emitter = DefaultEmitter::<usize>::new_with_span();
    emitter.naively_switch_states(true);
    Tokenizer::new_with_emitter(html, emitter).filter_map(|token| match token {
        Ok(Token::StartTag(tag)) => {
            let attributes = tag
                .attributes
                .iter()
                .map(|(key, value)| (lossy(key), lossy(value)))
                .collect();
            Some((lossy(&tag.name), attributes, tag.span.end))
        }
        _ => None,
    })
}

fn lossy(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

fn attribute(attributes: &Attributes, key: &str) -> Option<String> {
    attributes
        .iter()
        .find(|(name, _)| name == key)
        .map(|(_, value)| value.clone())
}

/// jsoup `hasClass`: a whitespace-separated class equal to `koboSpan`, ignoring case.
fn is_kobo_span(attributes: &Attributes) -> bool {
    attribute(attributes, "class").is_some_and(|classes| {
        classes
            .split([' ', '\t', '\n', '\x0c', '\r'])
            .any(|class| class.eq_ignore_ascii_case("koboSpan"))
    })
}

/// Converts increasing byte offsets of `text` to the UTF-16 offsets jsoup counts in.
struct Utf16Offsets<'text> {
    text: &'text str,
    byte: usize,
    utf16: usize,
}

impl<'text> Utf16Offsets<'text> {
    fn new(text: &'text str) -> Self {
        Self {
            text,
            byte: 0,
            utf16: 0,
        }
    }

    fn at(&mut self, byte: usize) -> usize {
        self.utf16 += self.text[self.byte..byte].encode_utf16().count();
        self.byte = byte;
        self.utf16
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn spans_end_after_their_start_tag_in_utf16_units() {
        let html = "<p>é😀<span class='x KOBOSPAN' id=\"kobo.1.1\">a</span>\
                    <span class=\"koboSpan\" id=\" \">b</span>\
                    <script><span class=\"koboSpan\" id=\"hidden\"></script>\
                    <!-- <span class=\"koboSpan\" id=\"comment\"> -->\
                    <div class=\"koboSpan\" id=\"div\"></div>";
        let first_end = "<p>é😀<span class='x KOBOSPAN' id=\"kobo.1.1\">"
            .encode_utf16()
            .count();
        assert_eq!(
            kobo_spans(html),
            vec![KoboSpan {
                id: "kobo.1.1".to_owned(),
                end: first_end,
            }],
            "blank ids, raw text, comments, and other elements are skipped"
        );
        assert!(has_kobo_span("<div class=\"koboSpan\"></div>"));
        assert!(!has_kobo_span(
            "<style>.koboSpan{}</style><p class=\"kobo\">"
        ));
    }
}
