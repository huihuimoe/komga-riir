//! MOBI HTML chapter splitting, `<guide>` TOC fallback, and `html5gum`-based XHTML normalization
//! with `<img recindex>` and `<a filepos>` rewriting.

use std::collections::HashMap;
use std::fmt::Write as _;

use html5gum::{DefaultEmitter, EndTag, SpanBound, StartTag, Token, Tokenizer};

/// A raw chapter slice delimited by `<mbp:pagebreak>` with its byte range in the decoded MOBI HTML.
#[derive(Debug, Clone)]
pub(super) struct RawChapter {
    pub(super) start: usize,
    pub(super) end: usize,
    pub(super) html: String,
}

/// Target chapter path and start byte offset for `filepos` link resolution.
#[derive(Debug, Clone)]
pub(super) struct ChapterSpan {
    pub(super) start: usize,
    pub(super) path: String,
}

const VOID_ELEMENTS: &[&str] = &[
    "area", "base", "br", "col", "embed", "hr", "img", "input", "link", "meta", "param", "source",
    "track", "wbr",
];

fn find_ascii_ci(haystack: &str, needle: &[u8]) -> Option<usize> {
    haystack
        .as_bytes()
        .windows(needle.len())
        .position(|window| window.eq_ignore_ascii_case(needle))
}

/// Extracts the `<body>...</body>` inner HTML and its starting byte offset in `document`.
pub(super) fn extract_body(document: &str) -> (&str, usize) {
    let start = find_ascii_ci(document, b"<body")
        .and_then(|pos| document[pos..].find('>').map(|end| pos + end + 1))
        .unwrap_or(0);
    let end =
        find_ascii_ci(&document[start..], b"</body>").map_or(document.len(), |pos| start + pos);
    (&document[start..end], start)
}

/// Splits `body` on `<mbp:pagebreak>` tags while tracking byte offsets in the full MOBI document.
pub(super) fn split_chapters(body: &str, body_offset: usize) -> Vec<RawChapter> {
    let mut chapters = Vec::new();
    let mut content_start = 0;

    while let Some(rel) = find_ascii_ci(&body[content_start..], b"<mbp:pagebreak") {
        let marker_start = content_start + rel;
        let Some(open_end_rel) = body[marker_start..].find('>') else {
            break;
        };
        let open_end = marker_start + open_end_rel + 1;
        let self_closing = body[marker_start..open_end - 1].trim_end().ends_with('/');
        let marker_end = if self_closing {
            open_end
        } else if let Some(close_rel) = find_ascii_ci(&body[open_end..], b"</mbp:pagebreak") {
            let close = open_end + close_rel;
            body[close..]
                .find('>')
                .map_or(open_end, |end| close + end + 1)
        } else {
            open_end
        };

        let slice = &body[content_start..marker_start];
        if has_visible_content(slice) {
            chapters.push(RawChapter {
                start: body_offset + content_start,
                end: body_offset + marker_start,
                html: slice.to_owned(),
            });
        }
        content_start = marker_end;
    }

    let tail = &body[content_start..];
    if has_visible_content(tail) {
        chapters.push(RawChapter {
            start: body_offset + content_start,
            end: body_offset + body.len(),
            html: tail.to_owned(),
        });
    }
    chapters
}

/// Parses `<guide>` `<reference filepos=...>` pointing to an inline HTML TOC page.
pub(super) fn parse_guide_toc(document: &str, chapters: &[RawChapter]) -> Vec<(usize, String)> {
    let Some(guide_start) = find_ascii_ci(document, b"<guide") else {
        return Vec::new();
    };
    let Some(guide_end_rel) = find_ascii_ci(&document[guide_start..], b"</guide>") else {
        return Vec::new();
    };
    let guide = &document[guide_start..guide_start + guide_end_rel + "</guide>".len()];
    let Some(toc_pos) = find_guide_toc_filepos(guide) else {
        return Vec::new();
    };
    let Some(toc_chapter) = chapters
        .iter()
        .find(|chapter| chapter.start <= toc_pos && toc_pos < chapter.end)
    else {
        return Vec::new();
    };
    parse_anchor_filepos_entries(&toc_chapter.html)
}

/// Resolves a MOBI byte offset to the matching chapter path (falling back to the last preceding
/// chapter or the first chapter). `spans` is sorted by `start`.
pub(super) fn chapter_for_offset(offset: usize, spans: &[ChapterSpan]) -> Option<&str> {
    let index = spans
        .partition_point(|span| span.start <= offset)
        .saturating_sub(1);
    spans.get(index).map(|span| span.path.as_str())
}

/// Normalizes a raw MOBI HTML fragment into a well-formed XHTML document, rewriting
/// `<img recindex>` and `<a filepos>` references to virtual EPUB paths.
pub(super) fn normalize_xhtml(
    title: &str,
    fragment: &str,
    image_paths: &HashMap<usize, String>,
    spans: &[ChapterSpan],
) -> String {
    let mut body = String::with_capacity(fragment.len() + 64);
    let mut open_tags: Vec<String> = Vec::new();
    let mut skip_script = false;
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);

    for token in Tokenizer::new_with_emitter(fragment, emitter).flatten() {
        match token {
            Token::StartTag(tag) => {
                handle_start_tag(
                    &tag,
                    image_paths,
                    spans,
                    &mut body,
                    &mut open_tags,
                    &mut skip_script,
                );
            }
            Token::EndTag(tag) => {
                handle_end_tag(&tag, &mut body, &mut open_tags, &mut skip_script);
            }
            Token::String(text) if !skip_script => {
                push_xml_escaped(&mut body, &String::from_utf8_lossy(&text.value), false);
            }
            _ => {}
        }
    }
    while let Some(tag) = open_tags.pop() {
        let _ = write!(body, "</{tag}>");
    }

    format!(
        r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head><meta charset="UTF-8"/><title>{}</title></head><body>{body}</body></html>"#,
        xml_escape_attr(title),
    )
}

fn handle_start_tag<S: SpanBound>(
    tag: &StartTag<S>,
    image_paths: &HashMap<usize, String>,
    spans: &[ChapterSpan],
    out: &mut String,
    open_tags: &mut Vec<String>,
    skip_script: &mut bool,
) {
    let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
    if name == "script" {
        *skip_script = !tag.self_closing;
        return;
    }
    if *skip_script || is_ignored_tag(&name) || !is_xml_name(&name) {
        return;
    }

    let is_void = tag.self_closing || VOID_ELEMENTS.contains(&name.as_str());
    out.push('<');
    out.push_str(&name);
    write_attributes(&name, tag, image_paths, spans, out);
    if is_void {
        out.push_str("/>");
    } else {
        out.push('>');
        open_tags.push(name);
    }
}

fn handle_end_tag<S: SpanBound>(
    tag: &EndTag<S>,
    out: &mut String,
    open_tags: &mut Vec<String>,
    skip_script: &mut bool,
) {
    let name = String::from_utf8_lossy(&tag.name).to_ascii_lowercase();
    if name == "script" {
        *skip_script = false;
        return;
    }
    if *skip_script || VOID_ELEMENTS.contains(&name.as_str()) || is_ignored_tag(&name) {
        return;
    }
    if let Some(pos) = open_tags.iter().rposition(|open| open == &name) {
        while open_tags.len() > pos {
            if let Some(closed) = open_tags.pop() {
                let _ = write!(out, "</{closed}>");
            }
        }
    }
}

fn write_attributes<S: SpanBound>(
    tag_name: &str,
    tag: &StartTag<S>,
    image_paths: &HashMap<usize, String>,
    spans: &[ChapterSpan],
    out: &mut String,
) {
    let mut rewritten_src: Option<String> = None;
    let mut rewritten_href: Option<String> = None;
    let mut attrs: Vec<(String, String)> = Vec::new();

    for (raw_key, raw_value) in &tag.attributes {
        let key = String::from_utf8_lossy(raw_key).to_ascii_lowercase();
        let value = String::from_utf8_lossy(raw_value).into_owned();
        if tag_name == "img" && matches!(key.as_str(), "recindex" | "mediarecindex") {
            if let Ok(index) = value.trim().parse::<usize>()
                && let Some(path) = image_paths.get(&index)
            {
                let relative = path.strip_prefix("OEBPS/").unwrap_or(path);
                rewritten_src = Some(format!("../{relative}"));
            }
            continue;
        }
        if tag_name == "a" && key == "filepos" {
            if let Ok(offset) = value.trim().parse::<usize>()
                && let Some(path) = chapter_for_offset(offset, spans)
            {
                let file_name = path.rsplit('/').next().unwrap_or(path);
                rewritten_href = Some(file_name.to_owned());
            }
            continue;
        }
        if is_xml_name(&key) && !key.starts_with("xmlns") && !attrs.iter().any(|(k, _)| k == &key) {
            attrs.push((key, value));
        }
    }

    if let Some(src) = rewritten_src {
        attrs.retain(|(key, _)| key != "src");
        attrs.push(("src".to_owned(), src));
    }
    if let Some(href) = rewritten_href {
        attrs.retain(|(key, _)| key != "href");
        attrs.push(("href".to_owned(), href));
    }
    attrs.sort_by(|left, right| left.0.cmp(&right.0));
    for (key, value) in attrs {
        let _ = write!(out, r#" {key}="{}""#, xml_escape_attr(&value));
    }
}

fn is_ignored_tag(name: &str) -> bool {
    name.starts_with("mbp:") || matches!(name, "html" | "head" | "body" | "guide" | "reference")
}

fn is_xml_name(name: &str) -> bool {
    let mut chars = name.chars();
    let Some(first) = chars.next() else {
        return false;
    };
    (first.is_ascii_alphabetic() || first == '_')
        && chars.all(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-' | '.'))
}

fn has_visible_content(html: &str) -> bool {
    let mut emitter = DefaultEmitter::default();
    emitter.naively_switch_states(true);
    for token in Tokenizer::new_with_emitter(html, emitter).flatten() {
        match token {
            Token::StartTag(tag) if tag.name.eq_ignore_ascii_case(b"img") => return true,
            Token::String(text) if !String::from_utf8_lossy(&text.value).trim().is_empty() => {
                return true;
            }
            _ => {}
        }
    }
    false
}

fn find_guide_toc_filepos(guide: &str) -> Option<usize> {
    let mut first_filepos = None;
    for token in Tokenizer::new(guide).flatten() {
        let Token::StartTag(tag) = token else {
            continue;
        };
        if !tag.name.eq_ignore_ascii_case(b"reference") {
            continue;
        }
        let filepos = tag
            .attributes
            .get(&b"filepos"[..])
            .and_then(|v| String::from_utf8_lossy(v).trim().parse::<usize>().ok());
        let is_toc = tag.attributes.get(&b"type"[..]).is_some_and(|v| {
            String::from_utf8_lossy(v)
                .trim()
                .eq_ignore_ascii_case("toc")
        });
        if is_toc && filepos.is_some() {
            return filepos;
        }
        if first_filepos.is_none() {
            first_filepos = filepos;
        }
    }
    first_filepos
}

fn parse_anchor_filepos_entries(html: &str) -> Vec<(usize, String)> {
    let mut entries = Vec::new();
    let mut active_pos: Option<usize> = None;
    let mut active_text = String::new();
    for token in Tokenizer::new(html).flatten() {
        match token {
            Token::StartTag(tag) if tag.name.eq_ignore_ascii_case(b"a") => {
                active_pos = tag
                    .attributes
                    .get(&b"filepos"[..])
                    .and_then(|v| String::from_utf8_lossy(v).trim().parse::<usize>().ok());
                active_text.clear();
            }
            Token::EndTag(tag) if tag.name.eq_ignore_ascii_case(b"a") => {
                if let Some(pos) = active_pos.take() {
                    let title = active_text.split_whitespace().collect::<Vec<_>>().join(" ");
                    if !title.is_empty() {
                        entries.push((pos, title));
                    }
                }
            }
            Token::String(text) if active_pos.is_some() => {
                active_text.push_str(&String::from_utf8_lossy(&text.value));
            }
            _ => {}
        }
    }
    entries
}

pub(super) fn xml_escape_attr(value: &str) -> String {
    let mut out = String::with_capacity(value.len());
    push_xml_escaped(&mut out, value, true);
    out
}

fn push_xml_escaped(out: &mut String, value: &str, escape_quotes: bool) {
    for ch in value.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' if escape_quotes => out.push_str("&quot;"),
            '\'' if escape_quotes => out.push_str("&apos;"),
            '\t'
            | '\n'
            | '\r'
            | '\u{20}'..='\u{d7ff}'
            | '\u{e000}'..='\u{fffd}'
            | '\u{10000}'..='\u{10ffff}' => out.push(ch),
            _ => {}
        }
    }
}
