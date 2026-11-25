//! Embedded image extraction, chapter assembly, and hierarchical TOC construction for MOBI books.

use std::collections::HashMap;

use komga_types::EpubTocEntry;

use super::header::MobiHeader;
use super::html::{self, ChapterSpan, RawChapter};
use super::indx::IndxTocEntry;
use super::{MobiChapter, MobiError, MobiImage};

/// Maximum number of embedded image records extracted per book.
const MAX_IMAGES: usize = 16_384;

pub(super) type ExtractedImages = (Vec<MobiImage>, HashMap<usize, String>, Option<usize>);

pub(super) fn cover_bytes(bytes: &[u8], header: &MobiHeader) -> Option<Vec<u8>> {
    let index = cover_record_index(bytes, header)?;
    header.record(bytes, index).map(<[u8]>::to_vec)
}

fn cover_record_index(bytes: &[u8], header: &MobiHeader) -> Option<usize> {
    let first = header.first_image_index?;
    let end = image_record_end(header);
    [header.cover_offset, header.thumb_offset]
        .into_iter()
        .flatten()
        .find_map(|offset| {
            let index = first.checked_add(offset)?;
            (index < end
                && header
                    .record(bytes, index)
                    .is_some_and(|record| sniff_image_type(record).is_some()))
            .then_some(index)
        })
}

pub(super) fn extract_images(
    bytes: &[u8],
    header: &MobiHeader,
) -> Result<ExtractedImages, MobiError> {
    let Some(first) = header.first_image_index else {
        return Ok((Vec::new(), HashMap::new(), None));
    };
    let end = image_record_end(header);
    let cover_rec = cover_record_index(bytes, header);

    let mut images = Vec::new();
    let mut recindex_map = HashMap::new();
    let mut cover_image_index = None;

    for record_index in first..end {
        let Some(record) = header.record(bytes, record_index) else {
            break;
        };
        if is_section_sentinel(record) {
            break;
        }
        let Some((media_type, ext)) = sniff_image_type(record) else {
            continue;
        };
        if images.len() >= MAX_IMAGES {
            return Err(MobiError::LimitExceeded("too many embedded images"));
        }
        let img_slot = images.len();
        let recindex = (record_index - first) + 1;
        let path = format!("OEBPS/images/asset-{recindex:04}.{ext}");
        if cover_rec == Some(record_index) {
            cover_image_index = Some(img_slot);
        }
        recindex_map.insert(recindex, path.clone());
        images.push(MobiImage {
            path,
            media_type,
            bytes: record.to_vec(),
        });
    }
    Ok((images, recindex_map, cover_image_index))
}

fn image_record_end(header: &MobiHeader) -> usize {
    let total = header.record_count();
    header
        .kf8_boundary
        .map_or(total, |boundary| boundary.min(total))
}

fn is_section_sentinel(record: &[u8]) -> bool {
    matches!(
        record.get(0..4),
        Some(b"FLIS" | b"FCIS" | b"FDST" | b"INDX" | b"DATP" | b"AUDI" | b"VIDE")
    ) || record == b"BOUNDARY"
        || record == b"\xe9\x8e\r\n"
}

fn sniff_image_type(bytes: &[u8]) -> Option<(&'static str, &'static str)> {
    if bytes.starts_with(&[0xff, 0xd8, 0xff]) {
        Some(("image/jpeg", "jpg"))
    } else if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        Some(("image/png", "png"))
    } else if bytes.starts_with(b"GIF87a") || bytes.starts_with(b"GIF89a") {
        Some(("image/gif", "gif"))
    } else if bytes.len() >= 12 && &bytes[0..4] == b"RIFF" && &bytes[8..12] == b"WEBP" {
        Some(("image/webp", "webp"))
    } else if bytes.starts_with(b"BM") && bytes.len() >= 14 {
        Some(("image/bmp", "bmp"))
    } else {
        None
    }
}

pub(super) fn build_chapter_spans(raw: &[RawChapter]) -> Vec<ChapterSpan> {
    raw.iter()
        .enumerate()
        .map(|(index, chapter)| ChapterSpan {
            start: chapter.start,
            path: format!("OEBPS/text/chapter-{index:04}.xhtml"),
        })
        .collect()
}

pub(super) fn build_chapters(
    raw: &[RawChapter],
    spans: &[ChapterSpan],
    images: &[MobiImage],
    cover_image_index: Option<usize>,
    recindex_map: &HashMap<usize, String>,
    indx_entries: &[IndxTocEntry],
    guide_entries: &[(usize, String)],
) -> Vec<MobiChapter> {
    let mut chapters = Vec::with_capacity(raw.len() + usize::from(cover_image_index.is_some()));
    if let Some(cover_idx) = cover_image_index
        && let Some(cover) = images.get(cover_idx)
    {
        let rel = cover.path.strip_prefix("OEBPS/").unwrap_or(&cover.path);
        let xhtml = format!(
            r#"<?xml version="1.0" encoding="UTF-8"?><!DOCTYPE html><html xmlns="http://www.w3.org/1999/xhtml"><head><meta charset="UTF-8"/><title>Cover</title></head><body><div class="cover"><img src="../{rel}" alt="Cover"/></div></body></html>"#
        );
        chapters.push(MobiChapter {
            title: "Cover".to_owned(),
            path: "OEBPS/text/cover.xhtml".to_owned(),
            xhtml,
        });
    }

    for (index, (chapter, span)) in raw.iter().zip(spans).enumerate() {
        let title = chapter_title(chapter, index, indx_entries, guide_entries);
        let xhtml = html::normalize_xhtml(&title, &chapter.html, recindex_map, spans);
        chapters.push(MobiChapter {
            title,
            path: span.path.clone(),
            xhtml,
        });
    }
    chapters
}

fn chapter_title(
    chapter: &RawChapter,
    index: usize,
    indx_entries: &[IndxTocEntry],
    guide_entries: &[(usize, String)],
) -> String {
    if let Some(entry) = indx_entries
        .iter()
        .find(|entry| chapter.start <= entry.offset && entry.offset < chapter.end)
    {
        return entry.label.clone();
    }
    if let Some((_, title)) = guide_entries
        .iter()
        .find(|(pos, _)| chapter.start <= *pos && *pos < chapter.end)
    {
        return title.clone();
    }
    format!("Chapter {}", index + 1)
}

pub(super) fn build_toc(
    indx_entries: &[IndxTocEntry],
    guide_entries: &[(usize, String)],
    spans: &[ChapterSpan],
    chapters: &[MobiChapter],
) -> Vec<EpubTocEntry> {
    if !indx_entries.is_empty() {
        let tree = build_indx_toc_tree(indx_entries, spans);
        if !tree.is_empty() {
            return tree;
        }
    }
    if !guide_entries.is_empty() {
        return guide_entries
            .iter()
            .filter_map(|(offset, title)| {
                let path = html::chapter_for_offset(*offset, spans)?;
                Some(EpubTocEntry {
                    title: title.clone(),
                    href: Some(path.to_owned()),
                    children: Vec::new(),
                })
            })
            .collect();
    }
    chapters
        .iter()
        .map(|chapter| EpubTocEntry {
            title: chapter.title.clone(),
            href: Some(chapter.path.clone()),
            children: Vec::new(),
        })
        .collect()
}

fn build_indx_toc_tree(entries: &[IndxTocEntry], spans: &[ChapterSpan]) -> Vec<EpubTocEntry> {
    let mut nodes: Vec<Option<EpubTocEntry>> = entries
        .iter()
        .map(|entry| {
            Some(EpubTocEntry {
                title: entry.label.clone(),
                href: html::chapter_for_offset(entry.offset, spans).map(str::to_owned),
                children: Vec::new(),
            })
        })
        .collect();

    if entries.iter().any(|entry| entry.parent.is_some()) {
        for index in (0..entries.len()).rev() {
            if let Some(parent) = entries[index].parent
                && parent < index
                && let Some(child) = nodes[index].take()
                && let Some(parent_node) = nodes[parent].as_mut()
            {
                parent_node.children.insert(0, child);
            }
        }
        return nodes.into_iter().flatten().collect();
    }

    let mut roots: Vec<EpubTocEntry> = Vec::new();
    let mut stack: Vec<(usize, EpubTocEntry)> = Vec::new();
    for (entry, node) in entries.iter().zip(nodes.into_iter().flatten()) {
        while let Some((depth, _)) = stack.last()
            && *depth >= entry.depth
        {
            let (_, finished) = stack.pop().expect("stack is non-empty");
            attach_node(&mut stack, &mut roots, finished);
        }
        stack.push((entry.depth, node));
    }
    while let Some((_, finished)) = stack.pop() {
        attach_node(&mut stack, &mut roots, finished);
    }
    roots
}

fn attach_node(
    stack: &mut [(usize, EpubTocEntry)],
    roots: &mut Vec<EpubTocEntry>,
    node: EpubTocEntry,
) {
    if let Some((_, parent)) = stack.last_mut() {
        parent.children.push(node);
    } else {
        roots.push(node);
    }
}
