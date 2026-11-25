//! Kotlin `OpdsController`'s entries and links.

use std::collections::HashMap;

use komga_types::{BookDetails, BookId, PageRequest, SeriesDetails};

use crate::mvc::media::format_size;
use crate::mvc::servlet_uri::{RequestContext, encode_path};
use crate::opds::dto::atom::{Entry, Link, NAVIGATION, PageStream};
use crate::opds::{adjacent_pages, server_time};

const STREAM_TYPES: [&str; 3] = ["image/jpeg", "image/png", "image/gif"];

/// The context URL every link starts with.
pub(super) struct Base(String);

impl Base {
    pub(super) fn of(context: &RequestContext) -> Self {
        Self(context.url())
    }

    pub(super) fn href(&self, path: &str) -> String {
        format!("{}/opds/v1.2/{}", self.0, encode_path(path))
    }

    pub(super) fn v2_catalog(&self) -> String {
        format!("{}/opds/v2/catalog", self.0)
    }

    pub(super) fn navigation(&self, rel: &'static str, path: &str) -> Link {
        Link::new(NAVIGATION, rel, self.href(path))
    }

    /// The `self` and `start` links, then `previous` and `next` around a page of `href`.
    pub(super) fn feed_links(&self, href: String, page: PageRequest, total: u64) -> Vec<Link> {
        let mut links = vec![
            Link::new(NAVIGATION, "self", href.clone()),
            self.navigation("start", "catalog"),
        ];
        links.extend(
            adjacent_pages(&href, page, total).map(|(rel, href)| Link::new(NAVIGATION, rel, href)),
        );
        links
    }

    pub(super) fn subsection(
        &self,
        title: String,
        updated: String,
        id: String,
        path: &str,
    ) -> Entry {
        Entry {
            title,
            updated,
            id,
            content: String::new(),
            authors: Vec::new(),
            links: vec![self.navigation("subsection", path)],
        }
    }

    pub(super) fn series(&self, series: SeriesDetails) -> Entry {
        let id = series.series.id.to_string();
        self.subsection(
            series.metadata.title,
            server_time(series.last_modified),
            id.clone(),
            &format!("series/{id}"),
        )
    }

    /// Kotlin `BookDto.toOpdsEntry`; `page_types` are the types its pages stream as.
    pub(super) fn book(&self, book: BookDetails, prefix: String, page_types: &[String]) -> Entry {
        let id = book.book.id.to_string();
        let path = book.book.url.to_path().expect("Books have file URLs");
        let extension = path
            .extension()
            .map(|it| it.to_string_lossy().to_lowercase());
        let file_name = path
            .file_name()
            .map(|it| it.to_string_lossy().replace(';', ""));
        let mut content = format!(
            "{} - {}",
            extension.unwrap_or_default(),
            format_size(book.book.file_size)
        );
        if !book.metadata.summary.trim().is_empty() {
            content.push_str(&format!("\n\n{}", book.metadata.summary));
        }
        let media_type = book.media.media_type.as_deref();
        let mut links = vec![
            Link::new(
                "image/jpeg",
                "http://opds-spec.org/image/thumbnail",
                self.href(&format!("books/{id}/thumbnail/small")),
            ),
            Link::new(
                "image/jpeg",
                "http://opds-spec.org/image",
                self.href(&format!("books/{id}/thumbnail")),
            ),
            Link::new(
                media_type.unwrap_or("application/octet-stream"),
                "http://opds-spec.org/acquisition",
                self.href(&format!(
                    "books/{id}/file/{}",
                    file_name.unwrap_or_default()
                )),
            ),
        ];
        if !page_types.is_empty() {
            let pages = self.href(&format!("books/{id}/pages/"));
            let (media_type, href) = match page_types {
                [single] if STREAM_TYPES.contains(&single.as_str()) => {
                    (single.as_str(), format!("{pages}{{pageNumber}}"))
                }
                _ => ("image/jpeg", format!("{pages}{{pageNumber}}?convert=jpeg")),
            };
            let progress = book.read_progress.as_ref();
            links.push(Link {
                stream: Some(PageStream {
                    count: book.media.page_count,
                    last_read: progress.map(|progress| progress.page),
                    last_read_date: progress.map(|progress| {
                        progress
                            .read_date
                            .strftime("%Y-%m-%dT%H:%M:%SZ")
                            .to_string()
                    }),
                }),
                ..Link::new(media_type, "http://vaemendis.net/opds-pse/stream", href)
            });
        }
        Entry {
            title: prefix + &book.metadata.title,
            updated: server_time(book.last_modified),
            id,
            content,
            authors: book
                .metadata
                .authors
                .into_iter()
                .map(|author| author.name)
                .collect(),
            links,
        }
    }

    pub(super) fn books(
        &self,
        books: Vec<BookDetails>,
        page_types: &HashMap<BookId, Vec<String>>,
        prefix: impl Fn(&BookDetails) -> String,
    ) -> Vec<Entry> {
        books
            .into_iter()
            .map(|book| {
                let types = page_types.get(&book.book.id).map_or(&[][..], Vec::as_slice);
                let prefix = prefix(&book);
                self.book(book, prefix, types)
            })
            .collect()
    }

    /// The OpenSearch URL template of the series search.
    pub(super) fn search_template(&self) -> String {
        format!("{}?search={{searchTerms}}", self.href("series"))
    }
}
