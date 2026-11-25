//! kepubify's `html.RenderWithOptions(RenderOptionAllowXMLDeclarations, RenderOptionPolyglot)`:
//! HTML that also reads as XHTML.

use html5ever::{Attribute, LocalName, QualName, local_name, ns};

use super::dom::{Dom, NodeData, NodeId};

/// Go `voidElements`.
const VOID: [&str; 15] = [
    "area", "base", "br", "col", "embed", "hr", "img", "input", "keygen", "link", "meta", "param",
    "source", "track", "wbr",
];

/// Elements whose text children Go writes unescaped.
const RAW_TEXT: [&str; 8] = [
    "iframe",
    "noembed",
    "noframes",
    "noscript",
    "plaintext",
    "script",
    "style",
    "xmp",
];

pub(super) fn render(dom: &Dom, id: NodeId, out: &mut String) {
    match &dom.nodes[id].data {
        NodeData::Document => {
            for &child in &dom.nodes[id].children {
                render(dom, child, out);
            }
        }
        NodeData::Text(text) => escape_text(text, out),
        // An XML declaration parses as a comment; it is written back as it was.
        NodeData::Comment(text) if text.starts_with("?xml ") && text.ends_with('?') => {
            out.push('<');
            out.push_str(text);
            out.push('>');
        }
        NodeData::Comment(text) => {
            out.push_str("<!--");
            out.push_str(text);
            out.push_str("-->");
        }
        NodeData::Doctype {
            name,
            public_id,
            system_id,
        } => {
            out.push_str("<!DOCTYPE ");
            out.push_str(name);
            if !public_id.is_empty() {
                out.push_str(" PUBLIC ");
                quoted(public_id, out);
                if !system_id.is_empty() {
                    out.push(' ');
                    quoted(system_id, out);
                }
            } else if !system_id.is_empty() {
                out.push_str(" SYSTEM ");
                quoted(system_id, out);
            }
            out.push('>');
        }
        NodeData::Element { name, attrs } => render_element(dom, id, name, attrs, out),
    }
}

fn render_element(dom: &Dom, id: NodeId, name: &QualName, attrs: &[Attribute], out: &mut String) {
    let tag = &*name.local;
    out.push('<');
    out.push_str(tag);
    for attr in attrs.iter().chain(&polyglot_attributes(&name.local, attrs)) {
        out.push(' ');
        // html5ever gives a foreign `xmlns` an empty prefix; Go gives it none.
        if let Some(prefix) = attr
            .name
            .prefix
            .as_ref()
            .filter(|prefix| !prefix.is_empty())
        {
            out.push_str(prefix);
            out.push(':');
        }
        out.push_str(&attr.name.local);
        out.push_str("=\"");
        escape(&attr.value, out);
        out.push('"');
    }
    if VOID.contains(&tag) {
        out.push_str("/>");
        return;
    }
    out.push('>');
    let children = &dom.nodes[id].children;
    if matches!(tag, "pre" | "listing" | "textarea")
        && let Some(NodeData::Text(text)) = children.first().map(|&child| &dom.nodes[child].data)
        && text.starts_with('\n')
    {
        out.push('\n');
    }
    for &child in children {
        match &dom.nodes[child].data {
            NodeData::Text(text) if RAW_TEXT.contains(&tag) => out.push_str(text),
            _ => render(dom, child, out),
        }
    }
    // Go stops rendering after `plaintext`; the parser leaves nothing after it to render.
    out.push_str("</");
    out.push_str(tag);
    out.push('>');
}

/// The namespace and type attributes polyglot rendering adds when missing. An attribute counts
/// as present when its prefix is the wanted name, as in the fork's `ensureAttr`.
fn polyglot_attributes(tag: &LocalName, attrs: &[Attribute]) -> Vec<Attribute> {
    let wanted: &[(Option<&str>, &str, &str)] = match *tag {
        local_name!("html") => &[(None, "xmlns", "http://www.w3.org/1999/xhtml")],
        local_name!("svg") => &[
            (None, "xmlns", "http://www.w3.org/2000/svg"),
            (Some("xmlns"), "xlink", "http://www.w3.org/1999/xlink"),
        ],
        local_name!("math") => &[(None, "xmlns", "http://www.w3.org/1998/Math/MathML")],
        local_name!("script") => &[(None, "type", "text/javascript")],
        local_name!("style") => &[(None, "type", "text/css")],
        _ => &[],
    };
    let mut added: Vec<Attribute> = Vec::new();
    for &(prefix, key, value) in wanted {
        let present = attrs.iter().chain(&added).any(|attr| {
            let attr_prefix = attr.name.prefix.as_deref().unwrap_or("");
            (attr_prefix.eq_ignore_ascii_case(prefix.unwrap_or(""))
                && (*attr.name.local).eq_ignore_ascii_case(key))
                || attr_prefix.eq_ignore_ascii_case(key)
        });
        if !present {
            added.push(Attribute {
                name: QualName::new(prefix.map(Into::into), ns!(), LocalName::from(key)),
                value: value.into(),
            });
        }
    }
    added
}

/// Go `escape` with polyglot's `&#160;` for no-break spaces.
fn escape_text(text: &str, out: &mut String) {
    for (index, part) in text.split('\u{a0}').enumerate() {
        if index > 0 {
            out.push_str("&#160;");
        }
        escape(part, out);
    }
}

/// Go `escape`: `&'<>"` and carriage returns.
fn escape(text: &str, out: &mut String) {
    for char in text.chars() {
        match char {
            '&' => out.push_str("&amp;"),
            '\'' => out.push_str("&#39;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&#34;"),
            '\r' => out.push_str("&#13;"),
            other => out.push(other),
        }
    }
}

/// Go `writeQuoted`: single quotes when the value has a double quote.
fn quoted(value: &str, out: &mut String) {
    let quote = if value.contains('"') { '\'' } else { '"' };
    out.push(quote);
    out.push_str(value);
    out.push(quote);
}
