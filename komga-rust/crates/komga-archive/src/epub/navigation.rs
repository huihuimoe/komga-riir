//! Kotlin `Nav.kt`, `Ncx.kt`, and `processOpfGuide`: navigation entries with hrefs resolved to
//! member names.

use komga_types::EpubTocEntry;

use super::href::{self, MalformedHref};
use super::xml::{self, Element};

/// Kotlin `Epub3Nav`: the `epub:type` of a nav element.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Epub3Nav {
    Toc,
    Landmarks,
    PageList,
}

impl Epub3Nav {
    fn value(self) -> &'static str {
        match self {
            Self::Toc => "toc",
            Self::Landmarks => "landmarks",
            Self::PageList => "page-list",
        }
    }
}

/// Kotlin `Epub2Nav`: the list and entry tags of an NCX section.
#[derive(Debug, Clone, Copy)]
pub(crate) enum Epub2Nav {
    Toc,
    PageList,
}

impl Epub2Nav {
    fn tags(self) -> (&'static str, &'static str) {
        match self {
            Self::Toc => ("navMap", "navPoint"),
            Self::PageList => ("pageList", "pageTarget"),
        }
    }
}

/// Kotlin `processNav`: the entries of the first `nav` whose `*type` attribute is `nav_type`.
/// `path` is the member name of the nav document.
pub(crate) fn process_nav(
    content: &[u8],
    path: &str,
    nav_type: Epub3Nav,
) -> Result<Vec<EpubTocEntry>, MalformedHref> {
    let document = xml::parse(content);
    let dir = href::parent(path);
    let nav = document.descendants().into_iter().find(|(_, element)| {
        element.is("nav")
            && element
                .attributes()
                .any(|(key, value)| key.ends_with("type") && value == nav_type.value())
    });
    let Some((_, nav)) = nav else {
        return Ok(Vec::new());
    };
    let mut entries = Vec::new();
    for item in list_items(nav) {
        entries.extend(nav_entry(item, dir.as_deref())?);
    }
    Ok(entries)
}

/// jsoup `select(":root > ol > li")`.
fn list_items(element: &Element) -> impl Iterator<Item = &Element> {
    element
        .children()
        .filter(|child| child.is("ol"))
        .flat_map(|list| list.children().filter(|child| child.is("li")))
}

fn nav_entry(item: &Element, dir: Option<&str>) -> Result<Option<EpubTocEntry>, MalformedHref> {
    // jsoup `selectFirst(":root > a, span")`: a child link or any descendant span.
    let title = item.descendants().into_iter().find(|&(parent, element)| {
        element.is("span") || (element.is("a") && parent.is_some_and(|up| std::ptr::eq(up, item)))
    });
    let link = item.children().find(|child| child.is("a"));
    let href = link
        .map(|link| href::url_decode(link.attr("href")))
        .transpose()?;
    let mut children = Vec::new();
    for child in list_items(item) {
        children.extend(nav_entry(child, dir)?);
    }
    Ok(title.map(|(_, title)| EpubTocEntry {
        title: title.text(),
        href: href.map(|href| href::normalize_href(dir, &href)),
        children,
    }))
}

/// Kotlin `processNcx`: the entries of `nav_type`'s list in the NCX document at `path`.
pub(crate) fn process_ncx(
    content: &[u8],
    path: &str,
    nav_type: Epub2Nav,
) -> Result<Vec<EpubTocEntry>, MalformedHref> {
    let document = xml::parse(content);
    let dir = href::parent(path);
    let (list, entry) = nav_type.tags();
    let mut entries = Vec::new();
    for point in document.select_child_of(|parent| parent.is(list), |child| child.is(entry)) {
        entries.extend(ncx_entry(point, entry, dir.as_deref())?);
    }
    Ok(entries)
}

fn ncx_entry(
    point: &Element,
    tag: &str,
    dir: Option<&str>,
) -> Result<Option<EpubTocEntry>, MalformedHref> {
    // Both selections search the whole subtree, nested entries included, as jsoup does.
    let title = point
        .select_child_of(|parent| parent.is("navLabel"), |child| child.is("text"))
        .first()
        .map(|text| text.text());
    let content = point
        .descendants()
        .into_iter()
        .find(|(_, element)| element.is("content"));
    let href = content
        .map(|(_, content)| href::url_decode(content.attr("src")))
        .transpose()?;
    let mut children = Vec::new();
    for child in point.children().filter(|child| child.is(tag)) {
        children.extend(ncx_entry(child, tag, dir)?);
    }
    Ok(title.map(|title| EpubTocEntry {
        title,
        href: href.map(|href| href::normalize_href(dir, &href)),
        children,
    }))
}

/// Kotlin `processOpfGuide`: the references of the first `guide`, without children.
pub(crate) fn process_opf_guide(
    opf: &Element,
    opf_dir: Option<&str>,
) -> Result<Vec<EpubTocEntry>, MalformedHref> {
    let guide = opf
        .descendants()
        .into_iter()
        .find(|(_, element)| element.is_any_ns("guide"));
    let Some((_, guide)) = guide else {
        return Ok(Vec::new());
    };
    let references = guide
        .descendants()
        .into_iter()
        .filter(|(_, element)| element.is_any_ns("reference"));
    let mut entries = Vec::new();
    for (_, reference) in references {
        let href = reference.attr("href");
        let href = if href.trim().is_empty() {
            None
        } else {
            Some(href::normalize_href(opf_dir, &href::url_decode(href)?))
        };
        entries.push(EpubTocEntry {
            title: reference.attr("title").to_owned(),
            href,
            children: Vec::new(),
        });
    }
    Ok(entries)
}
