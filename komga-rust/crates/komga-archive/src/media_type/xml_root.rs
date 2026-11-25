//! Tika `XmlRootExtractor`: the namespace and local name of the root element, if the content
//! is XML up to that element.

use std::borrow::Cow;

use quick_xml::events::Event;
use quick_xml::name::ResolveResult;
use quick_xml::reader::NsReader;

/// `(namespace URI, local name)`, with an empty namespace for unqualified roots.
pub(super) fn root_element(data: &[u8]) -> Option<(String, String)> {
    let text = decode(data);
    let mut reader = NsReader::from_str(&text);
    loop {
        match reader.read_resolved_event().ok()? {
            (namespace, Event::Start(element) | Event::Empty(element)) => {
                let namespace = match namespace {
                    ResolveResult::Bound(namespace) => namespace.as_ref().to_owned(),
                    ResolveResult::Unbound => String::new(),
                    // Java SAX rejects an undeclared prefix.
                    ResolveResult::Unknown(_) => return None,
                };
                return Some((namespace, element.local_name().as_ref().to_owned()));
            }
            (_, Event::Decl(_) | Event::Comment(_) | Event::PI(_) | Event::DocType(_)) => {}
            // Only whitespace may precede the root.
            (_, Event::Text(text)) if text.trim_ascii().is_empty() => {}
            _ => return None,
        }
    }
}

/// The content as UTF-8, which quick-xml requires. UTF-16 is recognized the way Java SAX does,
/// by its byte order mark or its leading `<`. Other content is read as ASCII-compatible
/// whatever it declares: every root in the registry is ASCII, so bytes that are not UTF-8
/// only need to become replacement characters, not decoded.
fn decode(data: &[u8]) -> Cow<'_, str> {
    let (big_endian, content) = match data {
        [0xfe, 0xff, rest @ ..] => (true, rest),
        [0xff, 0xfe, rest @ ..] => (false, rest),
        [0x00, b'<', ..] => (true, data),
        [b'<', 0x00, ..] => (false, data),
        _ => return String::from_utf8_lossy(data),
    };
    let pairs = content
        .iter()
        .step_by(2)
        .zip(content.iter().skip(1).step_by(2));
    let units = pairs.map(|(&first, &second)| {
        let pair = [first, second];
        if big_endian {
            u16::from_be_bytes(pair)
        } else {
            u16::from_le_bytes(pair)
        }
    });
    Cow::Owned(
        char::decode_utf16(units)
            .map(|char| char.unwrap_or(char::REPLACEMENT_CHARACTER))
            .collect(),
    )
}
