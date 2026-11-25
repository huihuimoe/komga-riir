//! Tika `MimeTypesReader` over the embedded registry and `MimeTypes.getMimeType(byte[])`.

use std::collections::HashMap;

use quick_xml::events::{BytesStart, Event};
use quick_xml::{Reader, XmlVersion};

use super::magic::{Clause, Header, Magic, Match, MatchSpec, Regexes};
use super::{text, xml_root};

const OCTET_STREAM: &str = "application/octet-stream";
const TEXT_PLAIN: &str = "text/plain";
const HTML: &str = "text/html";
const XML: &str = "application/xml";

pub(super) struct Registry {
    /// Normalized type names; magics and XML roots refer to them by index.
    names: Vec<String>,
    /// Type indexes by normalized name and by alias.
    indexes: HashMap<String, usize>,
    /// Tika `MimeType.getExtension` by type index: the first `*.ext` glob, with its dot.
    extensions: Vec<Option<String>>,
    /// In Tika `Magic` order.
    magics: Vec<Magic>,
    /// Types with root elements, by name, with their `(namespace, local name)` roots.
    xml_types: Vec<(usize, Vec<(String, String)>)>,
}

/// Tika `ClauseRecord`: a `<magic>` or `<match>` being read with its finished children.
struct Record {
    clause: Option<Clause>,
    min_should_match: Option<usize>,
    children: Vec<Clause>,
}

impl Registry {
    /// Reads the registry the way `MimeTypesReader` does; the embedded file is trusted.
    pub(super) fn parse(xml: &str) -> Self {
        let mut registry = Self {
            names: Vec::new(),
            indexes: HashMap::new(),
            extensions: Vec::new(),
            magics: Vec::new(),
            xml_types: Vec::new(),
        };
        let mut aliases: HashMap<String, usize> = HashMap::new();
        let mut roots: HashMap<usize, Vec<(String, String)>> = HashMap::new();
        let mut regexes = Regexes::new();
        let mut current: Option<usize> = None;
        let mut priority = 50;
        let mut records: Vec<Record> = Vec::new();
        let mut reader = Reader::from_str(xml);
        loop {
            let event = reader
                .read_event()
                .expect("The embedded registry is valid XML");
            let (element, empty) = match &event {
                Event::Start(element) => (element, false),
                Event::Empty(element) => (element, true),
                Event::End(element) => {
                    registry.end(
                        element.name().as_ref(),
                        &mut current,
                        priority,
                        &mut records,
                    );
                    continue;
                }
                Event::Eof => break,
                _ => continue,
            };
            let name = element.name();
            match (current, name.as_ref()) {
                (None, "mime-type") => {
                    let normalized = normalize(&attribute(element, "type").expect("Type name"));
                    let next = registry.names.len();
                    let index = *registry.indexes.entry(normalized.clone()).or_insert(next);
                    if index == next {
                        registry.names.push(normalized);
                        registry.extensions.push(None);
                    }
                    current = Some(index);
                }
                (Some(index), "alias") => {
                    let alias = normalize(&attribute(element, "type").expect("Alias name"));
                    aliases.insert(alias, index);
                }
                (Some(index), "glob") => {
                    if let Some(extension) = glob_extension(element) {
                        registry.extensions[index].get_or_insert(extension);
                    }
                }
                (Some(index), "root-XML") => roots.entry(index).or_default().push((
                    attribute(element, "namespaceURI").unwrap_or_default(),
                    attribute(element, "localName").unwrap_or_default(),
                )),
                (Some(_), "magic") => {
                    priority = attribute(element, "priority")
                        .filter(|value| !value.is_empty())
                        .map_or(50, |value| value.parse().expect("Magic priority"));
                    records.push(Record {
                        clause: None,
                        min_should_match: None,
                        children: Vec::new(),
                    });
                }
                (Some(index), "match") => {
                    records.push(registry.match_record(element, index, &mut regexes));
                }
                _ => {}
            }
            if empty {
                registry.end(name.as_ref(), &mut current, priority, &mut records);
            }
        }
        for (alias, index) in aliases {
            registry.indexes.entry(alias).or_insert(index);
        }
        let names = &registry.names;
        registry.magics.sort_by(|left, right| {
            left.compare(right, &names[left.type_index], &names[right.type_index])
        });
        let mut xml_types: Vec<_> = roots.into_iter().collect();
        xml_types.sort_by(|left, right| names[left.0].cmp(&names[right.0]));
        registry.xml_types = xml_types;
        registry
    }

    fn match_record(
        &self,
        element: &BytesStart<'_>,
        type_index: usize,
        regexes: &mut Regexes,
    ) -> Record {
        if let Some(min) = attribute(element, "minShouldMatch") {
            return Record {
                clause: None,
                min_should_match: Some(min.parse().expect("minShouldMatch count")),
                children: Vec::new(),
            };
        }
        let kind = attribute(element, "type").unwrap_or_else(|| "string".to_owned());
        let offset = attribute(element, "offset");
        let value = attribute(element, "value").expect("Match value");
        let mask = attribute(element, "mask");
        let matcher = Match::parse(
            &MatchSpec {
                type_name: &self.names[type_index],
                kind: &kind,
                offset: offset.as_deref(),
                value: &value,
                mask: mask.as_deref(),
            },
            regexes,
        );
        Record {
            clause: Some(Clause::Match(Box::new(matcher))),
            min_should_match: None,
            children: Vec::new(),
        }
    }

    /// `endElement`: closes a `<match>` into its parent, or a `<magic>` into the type's magics.
    fn end(
        &mut self,
        name: &str,
        current: &mut Option<usize>,
        priority: i32,
        records: &mut Vec<Record>,
    ) {
        let Some(type_index) = *current else {
            return;
        };
        match name {
            "mime-type" => *current = None,
            "match" => {
                let record = records.pop().expect("Open match");
                let clause = match (record.min_should_match, record.clause) {
                    (Some(min), _) => Clause::MinShouldMatch {
                        min,
                        clauses: record.children,
                    },
                    (None, Some(clause)) if record.children.is_empty() => clause,
                    (None, Some(clause)) => {
                        let mut children = record.children;
                        let nested = if children.len() == 1 {
                            children.pop().expect("One child")
                        } else {
                            Clause::Or(children)
                        };
                        Clause::And(vec![clause, nested])
                    }
                    (None, None) => unreachable!("A match record has a clause"),
                };
                records
                    .last_mut()
                    .expect("Parent record")
                    .children
                    .push(clause);
            }
            "magic" => {
                let record = records.pop().expect("Open magic");
                for clause in record.children {
                    self.magics.push(Magic::new(type_index, priority, clause));
                }
            }
            _ => {}
        }
    }

    /// `MimeTypes.forName(name).getExtension()` for a registered name or alias.
    pub(super) fn extension(&self, name: &str) -> Option<&str> {
        let index = *self.indexes.get(name)?;
        Some(self.extensions[index].as_deref().unwrap_or_default())
    }

    /// `MimeTypes.getMimeType(byte[])`, first result: the first magic that matches, with
    /// generic XML and HTML refined by root element; otherwise plain text or binary.
    pub(super) fn detect(&self, data: &[u8]) -> &str {
        if data.is_empty() {
            return OCTET_STREAM;
        }
        let header = Header::new(data);
        let Some(magic) = self.magics.iter().find(|magic| magic.clause.eval(&header)) else {
            return if text::looks_like_text(data) {
                TEXT_PLAIN
            } else {
                OCTET_STREAM
            };
        };
        let matched = self.names[magic.type_index].as_str();
        if matched != XML && matched != HTML {
            return matched;
        }
        match xml_root::root_element(data) {
            // Tika `RootXML.matches`, where a missing part only matches a missing part.
            Some(root) => self
                .xml_types
                .iter()
                .find(|(_, roots)| roots.contains(&root))
                .map_or(matched, |(index, _)| self.names[*index].as_str()),
            // TIKA-2419: not well-formed XML may still be HTML, else it is plain text.
            None if matched == XML => {
                let html = self.magics.iter().any(|magic| {
                    self.names[magic.type_index] == HTML && magic.clause.eval(&header)
                });
                if html { HTML } else { TEXT_PLAIN }
            }
            None => matched,
        }
    }
}

/// `Patterns.add`: only a literal `*` prefix makes an extension (with its dot); regexes do not.
fn glob_extension(element: &BytesStart<'_>) -> Option<String> {
    let pattern = attribute(element, "pattern").expect("Glob pattern");
    let regex = attribute(element, "isregex").is_some_and(|value| value == "true");
    pattern
        .strip_prefix('*')
        .filter(|extension| !regex && !extension.contains(['*', '?', '[']))
        .map(str::to_owned)
}

fn attribute(element: &BytesStart<'_>, name: &str) -> Option<String> {
    element
        .try_get_attribute(name)
        .expect("Registry attributes are well-formed")
        .map(|attribute| {
            attribute
                .normalized_value(XmlVersion::Explicit1_0)
                .expect("Registry attribute values unescape")
                .into_owned()
        })
}

/// Tika `MediaType.parse(..).toString()`: simple names stay as written; others are trimmed and
/// lowercased, with parameters sorted by lowercase key as `; key=value`.
fn normalize(name: &str) -> String {
    let simple = |part: &str| {
        !part.is_empty()
            && part
                .chars()
                .all(|char| matches!(char, '-' | '+' | '.' | '_' | '0'..='9' | 'a'..='z'))
    };
    if name
        .split_once('/')
        .is_some_and(|(kind, subtype)| simple(kind) && simple(subtype))
    {
        return name.to_owned();
    }
    let (base, parameters) = name.split_once(';').unwrap_or((name, ""));
    let (kind, subtype) = base.split_once('/').expect("Registry types have a subtype");
    let mut parameters: Vec<(String, &str)> = parameters
        .split(';')
        .filter_map(|parameter| {
            let (key, value) = parameter.split_once('=').unwrap_or((parameter, ""));
            let key = key.trim().to_lowercase();
            (!key.is_empty()).then(|| (key, value.trim().trim_matches(['"', '\''])))
        })
        .collect();
    parameters.sort();
    let mut normalized = format!(
        "{}/{}",
        kind.trim().to_lowercase(),
        subtype.trim().to_lowercase()
    );
    for (key, value) in parameters {
        normalized.push_str(&format!("; {key}={value}"));
    }
    normalized
}
