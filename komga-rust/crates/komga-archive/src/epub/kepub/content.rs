//! kepubify v4.0.4 `TransformContent` with its default options: the KEPUB form of an EPUB
//! content document, whose `koboSpan` elements Kobo readers locate reading positions by.
//!
//! The document is parsed and rendered as kepubify's fork of Go's `x/net/html` does: HTML5
//! with lenient self-closing tags, written back as polyglot HTML with the XML declaration kept.

use html5ever::{Attribute, LocalName, QualName, local_name, ns};

use super::dom::{self, DOCUMENT, Dom, NodeData, NodeId};
use super::{render, sentences};

/// The KEPUB form of a content document. Bytes that are not UTF-8 are dropped, as kepubify's
/// cleanup removes the replacement characters they decode to.
#[must_use]
pub(crate) fn transform_content(content: &[u8]) -> String {
    let mut dom = dom::parse(&String::from_utf8_lossy(content));
    charset_utf8(&mut dom);
    kobo_styles(&mut dom);
    kobo_divs(&mut dom);
    kobo_spans(&mut dom);
    clean(&mut dom);
    let mut out = String::with_capacity(content.len() * 2);
    render::render(&dom, DOCUMENT, &mut out);
    out
}

/// kepubify `transformDummyTitlepageRequired` for the first linear spine document: a page with
/// more than four paragraphs or twenty long words, no images and almost no text, or more than four
/// images is not a title page, so Kobo would otherwise show it as one.
#[must_use]
pub(super) fn needs_dummy_titlepage(content: &[u8]) -> bool {
    let dom = dom::parse(&String::from_utf8_lossy(content));
    let (mut paragraphs, mut images, mut words) = (0, 0, 0);
    let mut stack = Vec::new();
    if let Some(body) = dom.find(DOCUMENT, &local_name!("body")) {
        push_children(&dom, body, &mut stack);
    }
    while let Some(id) = stack.pop() {
        match &dom.nodes[id].data {
            NodeData::Element { name, .. } => match name.local {
                local_name!("p") => {
                    paragraphs += 1;
                    if paragraphs > 4 {
                        return true;
                    }
                }
                local_name!("img") | local_name!("svg") => images += 1,
                local_name!("script")
                | local_name!("style")
                | local_name!("pre")
                | local_name!("audio")
                | local_name!("video")
                | local_name!("math") => {}
                _ => push_children(&dom, id, &mut stack),
            },
            NodeData::Text(text) => {
                words += text
                    .split_whitespace()
                    .filter(|word| word.len() > 3)
                    .count();
                if words > 20 {
                    return true;
                }
            }
            NodeData::Document | NodeData::Doctype { .. } | NodeData::Comment(_) => {}
        }
    }
    (images == 0 && words < 5) || images > 4
}

fn new_element(dom: &mut Dom, tag: LocalName, attrs: &[(&str, &str)]) -> NodeId {
    let attrs = attrs
        .iter()
        .map(|&(key, value)| Attribute {
            name: QualName::new(None, ns!(), LocalName::from(key)),
            value: value.into(),
        })
        .collect();
    dom.new_node(NodeData::Element {
        name: QualName::new(None, ns!(html), tag),
        attrs,
    })
}

fn new_text(dom: &mut Dom, parent: NodeId, text: &str) {
    let node = dom.new_node(NodeData::Text(text.to_owned()));
    dom.append(parent, node);
}

/// Children pushed in reverse, so that popping walks them in document order.
fn push_children(dom: &Dom, id: NodeId, stack: &mut Vec<NodeId>) {
    stack.extend(dom.nodes[id].children.iter().rev());
}

/// kepubify `transformContentCharsetUTF8`: a `head` meta declaring another charset now declares
/// UTF-8, since the content is written as UTF-8.
pub(super) fn charset_utf8(dom: &mut Dom) {
    let Some(head) = dom.find(DOCUMENT, &local_name!("head")) else {
        return;
    };
    let mut stack = vec![head];
    while let Some(id) = stack.pop() {
        push_children(dom, id, &mut stack);
        if dom.local_name(id) != Some(&local_name!("meta")) {
            continue;
        }
        let NodeData::Element { attrs, .. } = &mut dom.nodes[id].data else {
            continue;
        };
        let Some(index) = attrs.iter().position(|attr| {
            let key = &*attr.name.local;
            (key == "charset" && !attr.value.eq_ignore_ascii_case("UTF-8"))
                || (key == "http-equiv" && attr.value.eq_ignore_ascii_case("content-type"))
        }) else {
            continue;
        };
        if &*attrs[index].name.local == "charset" {
            attrs[index].value = "UTF-8".into();
        } else if let Some(content) = attrs.iter_mut().find(|attr| &*attr.name.local == "content")
            && let Some(utf8) = utf8_content_type(&content.value)
        {
            content.value = utf8.into();
        }
    }
}

/// Go `mime.ParseMediaType` then `FormatMediaType` with `charset=utf-8`, when the type declares
/// another charset.
fn utf8_content_type(value: &str) -> Option<String> {
    let mut parts = value.split(';');
    let media_type = parts.next()?.trim().to_ascii_lowercase();
    let mut params: Vec<(String, String)> = Vec::new();
    for part in parts.filter(|part| !part.trim().is_empty()) {
        let (key, value) = part.split_once('=')?;
        let value = value.trim();
        let value = value
            .strip_prefix('"')
            .and_then(|value| value.strip_suffix('"'))
            .unwrap_or(value);
        params.push((key.trim().to_ascii_lowercase(), value.to_owned()));
    }
    let charset = params.iter_mut().find(|(key, _)| key == "charset")?;
    if charset.1.eq_ignore_ascii_case("utf-8") {
        return None;
    }
    charset.1 = "utf-8".to_owned();
    params.sort();
    let mut formatted = media_type;
    for (key, value) in params {
        let token = !value.is_empty()
            && value
                .chars()
                .all(|char| char.is_ascii_graphic() && !"()<>@,;:\\\"/[]?=".contains(char));
        if token {
            formatted.push_str(&format!("; {key}={value}"));
        } else {
            let escaped = value.replace('\\', "\\\\").replace('"', "\\\"");
            formatted.push_str(&format!("; {key}=\"{escaped}\""));
        }
    }
    Some(formatted)
}

/// kepubify `transformContentKoboStyles`.
fn kobo_styles(dom: &mut Dom) {
    let Some(head) = dom.find(DOCUMENT, &local_name!("head")) else {
        return;
    };
    let style = new_element(
        dom,
        local_name!("style"),
        &[("type", "text/css"), ("class", "kobostylehacks")],
    );
    new_text(
        dom,
        style,
        "div#book-inner { margin-top: 0; margin-bottom: 0;}",
    );
    dom.append(head, style);
}

/// kepubify `transformContentKoboDivs`: the body content wrapped in
/// `div#book-columns > div#book-inner` unless already there.
pub(super) fn kobo_divs(dom: &mut Dom) {
    let Some(body) = dom.find(DOCUMENT, &local_name!("body")) else {
        return;
    };
    let has_divs = dom.nodes[body].children.iter().any(|&child| {
        dom.attr(child, "id") == Some("book-columns")
            && dom.nodes[child]
                .children
                .iter()
                .any(|&inner| dom.attr(inner, "id") == Some("book-inner"))
    });
    if has_divs {
        return;
    }
    for layer in ["book-inner", "book-columns"] {
        let div = new_element(dom, local_name!("div"), &[("id", layer)]);
        dom.reparent_children(body, div);
        dom.append(body, div);
    }
}

/// kepubify `transformContentKoboSpans`: each sentence of the body text in a
/// `span.koboSpan#kobo.{paragraph}.{segment}`, and each image in its own paragraph.
pub(super) fn kobo_spans(dom: &mut Dom) {
    let Some(body) = dom.find(DOCUMENT, &local_name!("body")) else {
        return;
    };
    if has_kobo_span(dom, body) {
        return;
    }
    let mut paragraph = 0;
    let mut segment = 0;
    let mut next_paragraph = false;
    let mut stack = vec![body];
    while let Some(id) = stack.pop() {
        match &dom.nodes[id].data {
            NodeData::Text(text) => {
                let text = text.clone();
                let parent = dom.nodes[id].parent.expect("Body text has a parent");
                let in_paragraph = dom.local_name(parent) == Some(&local_name!("p"));
                for sentence in sentences::split_sentences(&text) {
                    let node = if sentence.chars().all(char::is_whitespace) && !in_paragraph {
                        dom.new_node(NodeData::Text(sentence.to_owned()))
                    } else {
                        if next_paragraph {
                            paragraph += 1;
                            segment = 0;
                            next_paragraph = false;
                        }
                        segment += 1;
                        let span = kobo_span(dom, paragraph, segment);
                        new_text(dom, span, sentence);
                        span
                    };
                    dom.insert_before(id, node);
                }
                dom.detach(id);
            }
            NodeData::Element { name, .. } => match name.local {
                local_name!("img") => {
                    paragraph += 1;
                    segment = 1;
                    next_paragraph = false;
                    let span = kobo_span(dom, paragraph, segment);
                    dom.insert_before(id, span);
                    dom.append(span, id);
                }
                local_name!("script")
                | local_name!("style")
                | local_name!("pre")
                | local_name!("audio")
                | local_name!("video")
                | local_name!("svg")
                | local_name!("math") => {}
                ref tag => {
                    next_paragraph |= [
                        local_name!("p"),
                        local_name!("ol"),
                        local_name!("ul"),
                        local_name!("table"),
                        local_name!("h1"),
                        local_name!("h2"),
                        local_name!("h3"),
                        local_name!("h4"),
                        local_name!("h5"),
                        local_name!("h6"),
                    ]
                    .contains(tag);
                    push_children(dom, id, &mut stack);
                }
            },
            NodeData::Document | NodeData::Doctype { .. } | NodeData::Comment(_) => {}
        }
    }
}

/// kepubify `findClass(body, "koboSpan")`.
fn has_kobo_span(dom: &Dom, id: NodeId) -> bool {
    dom.nodes[id].children.iter().any(|&child| {
        dom.attr(child, "class").is_some_and(|class| {
            class
                .split([' ', '\t', '\r', '\n', '\x0c'])
                .any(|token| token == "koboSpan")
        }) || has_kobo_span(dom, child)
    })
}

fn kobo_span(dom: &mut Dom, paragraph: u32, segment: u32) -> NodeId {
    let id = format!("kobo.{paragraph}.{segment}");
    new_element(
        dom,
        local_name!("span"),
        &[("class", "koboSpan"), ("id", &id)],
    )
}

/// kepubify `transformContentClean`: no replacement characters, Adobe Adept metas, or empty
/// MS Word `o:p` and `st1:*` elements.
pub(super) fn clean(dom: &mut Dom) {
    let mut stack = vec![DOCUMENT];
    while let Some(id) = stack.pop() {
        if let NodeData::Text(text) = &mut dom.nodes[id].data {
            text.retain(|char| char != '\u{fffd}');
            continue;
        }
        let removed = dom.local_name(id).is_some_and(|tag| {
            let adept = *tag == local_name!("meta")
                && dom.attr(id, "name").is_some_and(|name| {
                    name == "Adept.expected.resource" || name == "Adept.resource"
                });
            adept || ((&**tag == "o:p" || tag.starts_with("st1:")) && is_empty(dom, id))
        });
        if removed {
            dom.detach(id);
        } else if !matches!(
            dom.nodes[id].data,
            NodeData::Doctype { .. } | NodeData::Comment(_)
        ) {
            push_children(dom, id, &mut stack);
        }
    }
}

/// kepubify `matchEmpty`: only comments and whitespace inside.
fn is_empty(dom: &Dom, id: NodeId) -> bool {
    dom.nodes[id]
        .children
        .iter()
        .all(|&child| match &dom.nodes[child].data {
            NodeData::Comment(_) => true,
            NodeData::Text(text) => text.chars().all(char::is_whitespace),
            _ => false,
        })
}
