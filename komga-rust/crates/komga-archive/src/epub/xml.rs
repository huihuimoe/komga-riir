//! The part of jsoup's `Parser.xmlParser()` Komga's EPUB code relies on: an element tree that
//! never fails, tag names compared lowercased, and `Element.text()`.
//!
//! jsoup recovers from any markup; quick-xml stops at markup it cannot tokenize, so such a
//! document keeps the elements read up to there.

use quick_xml::Reader;
use quick_xml::escape;
use quick_xml::events::{BytesStart, Event};

#[derive(Debug, Default)]
pub(crate) struct Element {
    /// The qualified name as written, prefix included.
    name: String,
    /// In document order; jsoup keeps the first of duplicated attributes.
    attributes: Vec<(String, String)>,
    children: Vec<Node>,
}

#[derive(Debug)]
enum Node {
    Element(Element),
    Text(String),
}

/// Parses `content`, skipping a byte order mark. Bytes that are not UTF-8 become replacement
/// characters.
pub(crate) fn parse(content: &[u8]) -> Element {
    let content = content.strip_prefix(b"\xef\xbb\xbf").unwrap_or(content);
    let text = String::from_utf8_lossy(content);
    let mut reader = Reader::from_str(&text);
    let config = reader.config_mut();
    config.check_end_names = false;
    config.allow_unmatched_ends = true;
    // The document node; its children are the top-level nodes.
    let mut stack = vec![Element::default()];
    loop {
        let event = match reader.read_event() {
            Ok(Event::Eof) | Err(_) => break,
            Ok(event) => event,
        };
        match event {
            Event::Start(start) => stack.push(element(&start)),
            Event::Empty(start) => push(&mut stack, Node::Element(element(&start))),
            Event::End(end) => {
                let name = end.name().as_ref().to_owned();
                // jsoup `popStackToClose`: an end tag without an open element is ignored.
                if let Some(depth) = stack.iter().skip(1).rposition(|open| open.name == name) {
                    while stack.len() > depth + 1 {
                        close(&mut stack);
                    }
                }
            }
            Event::Text(text) => push(&mut stack, Node::Text(text.to_string())),
            Event::CData(text) => push(&mut stack, Node::Text(text.to_string())),
            Event::GeneralRef(reference) => {
                let raw = format!("&{};", &*reference);
                push(&mut stack, Node::Text(unescape(&raw)));
            }
            Event::Comment(_) | Event::Decl(_) | Event::PI(_) | Event::DocType(_) | Event::Eof => {}
        }
    }
    while stack.len() > 1 {
        close(&mut stack);
    }
    stack.pop().expect("The document node stays at the bottom")
}

fn element(start: &BytesStart<'_>) -> Element {
    let mut attributes: Vec<(String, String)> = Vec::new();
    for attribute in start.html_attributes().with_checks(false).flatten() {
        let key = attribute.key.as_ref().to_owned();
        if attributes.iter().all(|(existing, _)| *existing != key) {
            attributes.push((key, unescape(&attribute.value)));
        }
    }
    Element {
        name: start.name().as_ref().to_owned(),
        attributes,
        children: Vec::new(),
    }
}

fn push(stack: &mut [Element], node: Node) {
    stack
        .last_mut()
        .expect("The document node stays at the bottom")
        .children
        .push(node);
}

fn close(stack: &mut Vec<Element>) {
    let element = stack.pop().expect("Only open elements are closed");
    push(stack, Node::Element(element));
}

/// Character references and XML or HTML named entities; others stay as written.
fn unescape(raw: &str) -> String {
    let mut text = String::with_capacity(raw.len());
    let mut rest = raw;
    while let Some(start) = rest.find('&') {
        text.push_str(&rest[..start]);
        rest = &rest[start..];
        let resolved = rest.find(';').and_then(|end| {
            let name = &rest[1..end];
            let value = match name.strip_prefix('#') {
                Some(number) => char_ref(number).map(String::from),
                None => escape::resolve_html5_entity(name).map(str::to_owned),
            };
            value.map(|value| (value, end + 1))
        });
        match resolved {
            Some((value, length)) => {
                text.push_str(&value);
                rest = &rest[length..];
            }
            None => {
                text.push('&');
                rest = &rest[1..];
            }
        }
    }
    text.push_str(rest);
    text
}

fn char_ref(number: &str) -> Option<char> {
    let code = match number.strip_prefix(['x', 'X']) {
        Some(hex) => u32::from_str_radix(hex, 16).ok()?,
        None => number.parse().ok()?,
    };
    char::from_u32(code).filter(|&char| char != '\0')
}

impl Element {
    /// jsoup `attr(key)`: the value, or empty when absent.
    pub(crate) fn attr(&self, key: &str) -> &str {
        self.attributes
            .iter()
            .find(|(name, _)| name == key)
            .map_or("", |(_, value)| value)
    }

    pub(crate) fn attributes(&self) -> impl Iterator<Item = (&str, &str)> {
        self.attributes
            .iter()
            .map(|(key, value)| (key.as_str(), value.as_str()))
    }

    /// jsoup selector `tag`: the lowercased name.
    pub(crate) fn is(&self, tag: &str) -> bool {
        self.name.eq_ignore_ascii_case(tag)
    }

    /// jsoup selector `*|tag`: the tag in any namespace prefix.
    pub(crate) fn is_any_ns(&self, tag: &str) -> bool {
        let local = self
            .name
            .rsplit_once(':')
            .map_or(&*self.name, |(_, local)| local);
        self.is(tag) || local.eq_ignore_ascii_case(tag)
    }

    pub(crate) fn children(&self) -> impl Iterator<Item = &Element> {
        self.children.iter().filter_map(|node| match node {
            Node::Element(element) => Some(element),
            Node::Text(_) => None,
        })
    }

    /// This element and its descendants in document order, each with its parent.
    pub(crate) fn descendants(&self) -> Vec<(Option<&Element>, &Element)> {
        let mut found = vec![(None, self)];
        self.collect_descendants(&mut found);
        found
    }

    fn collect_descendants<'tree>(
        &'tree self,
        found: &mut Vec<(Option<&'tree Element>, &'tree Element)>,
    ) {
        for child in self.children() {
            found.push((Some(self), child));
            child.collect_descendants(found);
        }
    }

    /// jsoup `select("parent > child")` with the given tests, in document order.
    pub(crate) fn select_child_of(
        &self,
        parent: impl Fn(&Element) -> bool,
        child: impl Fn(&Element) -> bool,
    ) -> Vec<&Element> {
        self.descendants()
            .into_iter()
            .filter(|&(up, element)| up.is_some_and(&parent) && child(element))
            .map(|(_, element)| element)
            .collect()
    }

    /// jsoup `Element.text()`: descendant text with whitespace collapsed and trimmed; `br`
    /// separates words.
    pub(crate) fn text(&self) -> String {
        let mut text = String::new();
        self.append_text(&mut text);
        text.trim_matches(|char: char| char <= ' ').to_owned()
    }

    fn append_text(&self, text: &mut String) {
        for node in &self.children {
            match node {
                Node::Text(content) => append_normalized(text, content),
                Node::Element(element) => {
                    if element.is("br") && !text.ends_with(' ') {
                        text.push(' ');
                    }
                    element.append_text(text);
                }
            }
        }
    }
}

/// jsoup `StringUtil.appendNormalisedWhitespace` after the text gathered so far.
fn append_normalized(text: &mut String, content: &str) {
    for char in content.chars() {
        if matches!(char, ' ' | '\t' | '\n' | '\x0c' | '\r' | '\u{a0}') {
            if !text.is_empty() && !text.ends_with(' ') {
                text.push(' ');
            }
        } else if !matches!(char, '\u{200b}' | '\u{ad}') {
            text.push(char);
        }
    }
}
