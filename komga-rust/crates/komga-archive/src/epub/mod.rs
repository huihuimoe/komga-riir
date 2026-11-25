//! Kotlin `Epub.kt`, `Opf.kt`, and the structure half of `EpubExtractor`: the package document,
//! resources, reading order, and navigation of an EPUB file.
//!
//! The manifest and the spine stay apart: resources list spine items first, in spine order, as
//! pages, then the remaining manifest items as assets. An `idref` naming no manifest item is
//! skipped, as in Kotlin, rather than failing the book.

mod divina;
mod href;
pub(crate) mod kepub;
mod metadata;
mod navigation;
pub(crate) mod positions;
mod xml;

use std::collections::HashMap;
use std::fs::File;
use std::io::BufReader;
use std::path::Path;

use komga_types::{EpubTocEntry, MediaFileSubType};

use self::navigation::{Epub2Nav, Epub3Nav};
use self::xml::Element;
use crate::zip::{self, ZipArchive, ZipError};

pub use self::href::{MalformedHref, url_decode};
pub use self::metadata::{EpubCreator, EpubMetadata};

#[derive(Debug, thiserror::Error)]
pub enum EpubError {
    #[error(transparent)]
    Zip(#[from] ZipError),
    #[error("META-INF/container.xml does not contain rootfile tag")]
    MissingRootfile,
    #[error("Could not open OPF resource")]
    MissingOpf,
    #[error(transparent)]
    MalformedHref(#[from] MalformedHref),
    #[error("No member named {0}")]
    MissingMember(String),
    #[error("Malformed OPF: {0}")]
    MalformedOpf(String),
}

/// Kotlin `ManifestItem`.
#[derive(Debug, Clone, PartialEq, Eq)]
struct ManifestItem {
    id: String,
    href: String,
    media_type: String,
    properties: Vec<String>,
}

/// Kotlin `MediaFile` of an EPUB resource.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EpubResource {
    /// The member name, with the href's anchor if any.
    pub file_name: String,
    pub media_type: String,
    pub sub_type: MediaFileSubType,
    /// `None` when no member has that name.
    pub file_size: Option<u64>,
}

/// Kotlin `EpubPackage`: the opened archive and its parsed package document.
pub struct EpubPackage {
    archive: ZipArchive<BufReader<File>>,
    opf: Element,
    opf_path: String,
    opf_dir: Option<String>,
    /// Kotlin `associate` by id: first-seen order, last-seen item.
    manifest: Vec<ManifestItem>,
    manifest_index: HashMap<String, usize>,
}

/// Kotlin `EpubExtractor.isEpub`: the `mimetype` member says `application/epub+zip`.
#[must_use]
pub fn is_epub(path: &Path) -> bool {
    zip::read_entry(path, "mimetype")
        .is_ok_and(|bytes| String::from_utf8_lossy(&bytes).trim() == "application/epub+zip")
}

impl EpubPackage {
    /// Kotlin `Path.epub`: reads the package document `META-INF/container.xml` points to.
    ///
    /// # Errors
    /// An unreadable archive or member, no `rootfile`, or no member at its `full-path`.
    pub fn open(path: &Path) -> Result<Self, EpubError> {
        let mut archive = ZipArchive::new(BufReader::new(File::open(path).map_err(ZipError::Io)?))?;
        let container = archive
            .read_named("META-INF/container.xml")?
            .ok_or(EpubError::MissingRootfile)?;
        let container = xml::parse(&container);
        let opf_path = container
            .descendants()
            .into_iter()
            .find(|(_, element)| element.is("rootfile"))
            .map(|(_, rootfile)| rootfile.attr("full-path").to_owned())
            .ok_or(EpubError::MissingRootfile)?;
        let opf = archive
            .read_named(&opf_path)?
            .ok_or(EpubError::MissingOpf)?;
        let opf = xml::parse(&opf);
        let (manifest, manifest_index) = manifest(&opf);
        Ok(Self {
            archive,
            opf,
            opf_dir: href::parent(&opf_path),
            opf_path,
            manifest,
            manifest_index,
        })
    }

    fn opf_dir(&self) -> Option<&str> {
        self.opf_dir.as_deref()
    }

    fn manifest_item(&self, id: &str) -> Option<&ManifestItem> {
        self.manifest_index
            .get(id)
            .map(|&index| &self.manifest[index])
    }

    /// The manifest items the spine refers to, in spine order.
    fn spine(&self) -> Vec<&ManifestItem> {
        self.opf
            .select_child_of(
                |parent| parent.is_any_ns("spine"),
                |child| child.is_any_ns("itemref"),
            )
            .into_iter()
            .filter_map(|itemref| self.manifest_item(itemref.attr("idref")))
            .collect()
    }

    fn member_size(&self, name: &str) -> Option<u64> {
        self.archive
            .entries()
            .iter()
            .find(|entry| entry.name() == name)
            .map(zip::Entry::size)
    }

    /// Kotlin `getResources`: spine items as pages, then the other manifest items as assets.
    ///
    /// # Errors
    /// An href with a malformed `%` escape.
    pub fn resources(&self) -> Result<Vec<EpubResource>, MalformedHref> {
        let spine = self.spine();
        let assets = self
            .manifest
            .iter()
            .filter(|item| !spine.iter().any(|page| page.id == item.id));
        let typed = spine
            .iter()
            .map(|&item| (item, MediaFileSubType::EpubPage))
            .chain(assets.map(|item| (item, MediaFileSubType::EpubAsset)));
        let mut resources = Vec::new();
        for (item, sub_type) in typed {
            let file_name = href::normalize_href(self.opf_dir(), &href::url_decode(&item.href)?);
            resources.push(EpubResource {
                file_size: self.member_size(&file_name),
                file_name,
                media_type: item.media_type.clone(),
                sub_type,
            });
        }
        Ok(resources)
    }

    /// Kotlin `getCover`: the data of the EPUB 3 `cover-image` item, else of the item the EPUB 2
    /// `cover` meta names, else of the item with id `cover-image`; `None` without such an item
    /// or member.
    ///
    /// # Errors
    /// An unreadable member or a malformed href.
    pub fn cover(&mut self) -> Result<Option<Vec<u8>>, EpubError> {
        let epub2 = || {
            let metas = self.opf.select_child_of(
                |parent| parent.is_any_ns("metadata"),
                |child| child.is_any_ns("meta"),
            );
            // jsoup `[name=cover]` trims and ignores case.
            let content = metas
                .iter()
                .find(|meta| meta.attr("name").trim().eq_ignore_ascii_case("cover"))?
                .attr("content");
            (!content.trim().is_empty())
                .then(|| self.manifest_item(content))
                .flatten()
        };
        let item = self
            .manifest
            .iter()
            .find(|item| item.properties.iter().any(|p| p == "cover-image"))
            .or_else(epub2)
            .or_else(|| self.manifest.iter().find(|item| item.id == "cover-image"));
        let Some(item) = item else {
            return Ok(None);
        };
        let name = href::normalize_href(self.opf_dir(), &href::url_decode(&item.href)?);
        Ok(self.archive.read_named(&name)?)
    }

    /// Kotlin `computePageCount`: one page per started KiB of compressed spine members. Spine
    /// hrefs are not URL-decoded here, as in Kotlin.
    #[must_use]
    pub fn page_count(&self) -> u64 {
        let spine: Vec<String> = self
            .spine()
            .iter()
            .map(|item| href::normalize_href(self.opf_dir(), &item.href))
            .collect();
        self.archive
            .entries()
            .iter()
            .filter(|entry| spine.iter().any(|name| name == entry.name()))
            .map(|entry| entry.compressed_size().div_ceil(1024))
            .sum()
    }

    /// Kotlin `isFixedLayout`: `rendition:layout` is `pre-paginated`, or the EPUB 2
    /// `fixed-layout` meta is `true`.
    #[must_use]
    pub fn is_fixed_layout(&self) -> bool {
        let metas = self.opf.select_child_of(
            |parent| parent.is_any_ns("metadata"),
            |child| child.is_any_ns("meta"),
        );
        let first = |key: &str, value: &str| {
            metas
                .iter()
                .find(|meta| meta.attr(key).trim().eq_ignore_ascii_case(value))
                .copied()
        };
        first("property", "rendition:layout").is_some_and(|meta| meta.text() == "pre-paginated")
            || first("name", "fixed-layout").is_some_and(|meta| meta.attr("content") == "true")
    }

    /// Kotlin `getToc`: the EPUB 3 nav when it has entries, else the NCX.
    ///
    /// # Errors
    /// An unreadable navigation member or a malformed href.
    pub fn toc(&mut self) -> Result<Vec<EpubTocEntry>, EpubError> {
        self.nav_then_ncx(Epub3Nav::Toc, Epub2Nav::Toc)
    }

    /// Kotlin `getPageList`: the EPUB 3 nav when it has entries, else the NCX.
    ///
    /// # Errors
    /// An unreadable navigation member or a malformed href.
    pub fn page_list(&mut self) -> Result<Vec<EpubTocEntry>, EpubError> {
        self.nav_then_ncx(Epub3Nav::PageList, Epub2Nav::PageList)
    }

    /// Kotlin `getLandmarks`: the EPUB 3 nav when it has entries, else the OPF guide.
    ///
    /// # Errors
    /// An unreadable navigation member or a malformed href.
    pub fn landmarks(&mut self) -> Result<Vec<EpubTocEntry>, EpubError> {
        if let Some(entries) = self
            .nav(Epub3Nav::Landmarks)?
            .filter(|entries| !entries.is_empty())
        {
            return Ok(entries);
        }
        Ok(navigation::process_opf_guide(&self.opf, self.opf_dir())?)
    }

    fn nav_then_ncx(
        &mut self,
        nav_type: Epub3Nav,
        ncx_type: Epub2Nav,
    ) -> Result<Vec<EpubTocEntry>, EpubError> {
        if let Some(entries) = self.nav(nav_type)?.filter(|entries| !entries.is_empty()) {
            return Ok(entries);
        }
        // Kotlin `getNcxResource`: the NCX media type first, then the usual ids.
        let ncx = self
            .manifest
            .iter()
            .find(|item| item.media_type == "application/x-dtbncx+xml")
            .or_else(|| {
                let ids = ["toc", "ncx", "ncxtoc"];
                self.manifest
                    .iter()
                    .find(|item| ids.contains(&item.id.as_str()))
            });
        let Some((path, content)) = self.navigation_member(ncx.map(|item| item.href.clone()))?
        else {
            return Ok(Vec::new());
        };
        Ok(navigation::process_ncx(&content, &path, ncx_type)?)
    }

    /// Kotlin `getNavResource` then `processNav`; `None` without a readable nav document.
    fn nav(&mut self, nav_type: Epub3Nav) -> Result<Option<Vec<EpubTocEntry>>, EpubError> {
        let nav = self
            .manifest
            .iter()
            .find(|item| item.properties.iter().any(|p| p == "nav"));
        let Some((path, content)) = self.navigation_member(nav.map(|item| item.href.clone()))?
        else {
            return Ok(None);
        };
        Ok(Some(navigation::process_nav(&content, &path, nav_type)?))
    }

    /// The member name and content a navigation href (not URL-decoded, as in Kotlin) names.
    fn navigation_member(
        &mut self,
        href: Option<String>,
    ) -> Result<Option<(String, Vec<u8>)>, ZipError> {
        let Some(href) = href else {
            return Ok(None);
        };
        let path = href::normalize_href(self.opf_dir(), &href);
        Ok(self
            .archive
            .read_named(&path)?
            .map(|content| (path, content)))
    }
}

/// Kotlin `getManifest`: `*|manifest > *|item`, and their positions by id.
fn manifest(opf: &Element) -> (Vec<ManifestItem>, HashMap<String, usize>) {
    let mut manifest: Vec<ManifestItem> = Vec::new();
    let mut index: HashMap<String, usize> = HashMap::new();
    let items = opf.select_child_of(
        |parent| parent.is_any_ns("manifest"),
        |child| child.is_any_ns("item"),
    );
    for item in items {
        let item = ManifestItem {
            id: item.attr("id").to_owned(),
            href: item.attr("href").to_owned(),
            media_type: item.attr("media-type").to_owned(),
            properties: item
                .attr("properties")
                .split(' ')
                .map(str::to_owned)
                .collect(),
        };
        match index.get(&item.id) {
            Some(&position) => manifest[position] = item,
            None => {
                index.insert(item.id.clone(), manifest.len());
                manifest.push(item);
            }
        }
    }
    (manifest, index)
}

#[cfg(test)]
mod tests;
