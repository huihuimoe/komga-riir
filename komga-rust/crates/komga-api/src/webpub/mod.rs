//! Kotlin `WebPubGenerator` and `OpdsGenerator`: WebPub manifests whose links point to the REST
//! API or to OPDS 2.

mod dto;
pub(crate) mod handlers;
mod metadata;

use komga_service::{ManifestSource, can_convert_to_jpeg};
use komga_types::{
    BookDetails, BookPage, EpubTocEntry, MediaFileSubType, MediaProfile, ReadingDirection,
};

use crate::mvc::servlet_uri;
use dto::{WpAuthenticate, WpLinkProperties, WpReadingProgression, WpRendition};
pub(crate) use dto::{WpLink, WpPublication};

const WEBPUB_JSON: &str = "application/webpub+json";
const DIVINA_JSON: &str = "application/divina+json";
pub(crate) const OPDS_JSON: &str = "application/opds+json";
pub(crate) const OPDS_PUBLICATION_JSON: &str = "application/opds-publication+json";
pub(crate) const OPDS_AUTHENTICATION_JSON: &str = "application/opds-authentication+json";
const PROGRESSION_JSON: &str = "application/vnd.readium.progression+json";
const PROGRESSION: &str = "http://www.cantook.com/api/progression";
const PROFILE_DIVINA: &str = "https://readium.org/webpub-manifest/profiles/divina";
const PROFILE_EPUB: &str = "https://readium.org/webpub-manifest/profiles/epub";
const PROFILE_PDF: &str = "https://readium.org/webpub-manifest/profiles/pdf";
const ACQUISITION: &str = "http://opds-spec.org/acquisition";
/// The `thumbnailType` bean.
const THUMBNAIL_TYPE: &str = "image/jpeg";
/// `recommendedImageMediaTypes`: pages in other formats get a JPEG alternate.
const RECOMMENDED_IMAGE_TYPES: [&str; 3] = ["image/jpeg", "image/png", "image/gif"];

/// `mediaProfileToWebPub`: the media type of the manifest of a profile.
pub(crate) fn media_type(profile: Option<MediaProfile>) -> &'static str {
    match profile {
        Some(MediaProfile::Divina) => DIVINA_JSON,
        Some(MediaProfile::Pdf | MediaProfile::Epub) | None => WEBPUB_JSON,
    }
}

/// Which API the links of a manifest point to.
#[derive(Clone, Copy, PartialEq, Eq)]
pub(crate) enum Generator {
    WebPub,
    /// Links also tell OPDS clients where to authenticate and where progression is kept.
    Opds,
}

/// `toManifestDivina`, `toManifestPdf`, or `toManifestEpub`, as `manifest` says; `context` is
/// the request's context URL.
pub(crate) fn manifest(
    generator: Generator,
    manifest: MediaProfile,
    source: &ManifestSource,
    context: &str,
) -> WpPublication {
    let api = Api {
        generator,
        context,
        book: &source.book,
        profile: source.profile,
    };
    let mut publication = api.publication();
    publication.metadata.conforms_to = Some(match manifest {
        MediaProfile::Divina => PROFILE_DIVINA,
        MediaProfile::Pdf => PROFILE_PDF,
        MediaProfile::Epub => PROFILE_EPUB,
    });
    publication
        .metadata
        .language
        .clone_from(&source.series_language);
    publication.metadata.reading_progression =
        source.reading_direction.map(|direction| match direction {
            ReadingDirection::LeftToRight => WpReadingProgression::Ltr,
            ReadingDirection::RightToLeft => WpReadingProgression::Rtl,
            ReadingDirection::Vertical | ReadingDirection::Webtoon => WpReadingProgression::Ttb,
        });
    publication.resources.push(api.thumbnail());
    match manifest {
        MediaProfile::Divina => {
            publication.reading_order = source
                .pages
                .iter()
                .enumerate()
                .map(|(index, page)| api.page_link(index + 1, page))
                .collect();
        }
        MediaProfile::Pdf => {
            publication.reading_order = (1..=api.book.media.page_count)
                .map(|number| WpLink {
                    href: Some(api.url(&format!("pages/{number}/raw"))),
                    media_type: Some("application/pdf".to_owned()),
                    ..WpLink::default()
                })
                .collect();
        }
        MediaProfile::Epub => epub(&mut publication, &api, source),
    }
    publication
}

/// The parts `toManifestEpub` adds.
fn epub(publication: &mut WpPublication, api: &Api<'_>, source: &ManifestSource) {
    let file_links = |sub_type| {
        source
            .files
            .iter()
            .filter(|file| file.sub_type == Some(sub_type))
            .map(|file| WpLink {
                href: Some(api.url(&format!("resource/{}", file.file_name))),
                media_type: file.media_type.clone(),
                ..WpLink::default()
            })
            .collect::<Vec<_>>()
    };
    let navigation = |entries: &[EpubTocEntry]| -> Vec<WpLink> {
        entries.iter().map(|entry| api.toc_link(entry)).collect()
    };
    publication.reading_order = file_links(MediaFileSubType::EpubPage);
    publication
        .resources
        .extend(file_links(MediaFileSubType::EpubAsset));
    if let Some(extension) = &source.extension {
        publication.metadata.rendition = Some(WpRendition {
            layout: if extension.is_fixed_layout {
                "fixed"
            } else {
                "reflowable"
            },
        });
        publication.toc = navigation(&extension.toc);
        publication.landmarks = navigation(&extension.landmarks);
        publication.page_list = navigation(&extension.page_list);
    }
}

/// `OpdsGenerator.toOpdsPublicationDto`: how OPDS 2 feeds list `book`.
pub(crate) fn publication(context: &str, book: &BookDetails) -> WpPublication {
    let api = Api {
        generator: Generator::Opds,
        context,
        book,
        profile: MediaProfile::from_media_type(book.media.media_type.as_deref()),
    };
    let mut publication = api.publication();
    publication.images.push(api.thumbnail());
    publication
}

/// URLs under `<context>/api/v1/` or `<context>/opds/v2/`.
pub(super) struct Api<'a> {
    generator: Generator,
    context: &'a str,
    pub(super) book: &'a BookDetails,
    profile: Option<MediaProfile>,
}

impl Api<'_> {
    /// `toBasePublicationDto`.
    fn publication(&self) -> WpPublication {
        WpPublication {
            context: "https://readium.org/webpub-manifest/context.jsonld",
            metadata: metadata::metadata(self.book, self.series_links()),
            links: self.links(),
            images: Vec::new(),
            reading_order: Vec::new(),
            resources: Vec::new(),
            toc: Vec::new(),
            landmarks: Vec::new(),
            page_list: Vec::new(),
        }
    }

    /// `uriBuilder.path("books/<id>/<path>").toUriString()`.
    fn url(&self, path: &str) -> String {
        self.root_url(&format!("books/{}/{path}", self.book.book.id.as_str()))
    }

    fn root_url(&self, path: &str) -> String {
        let root = match self.generator {
            Generator::WebPub => "/api/v1",
            Generator::Opds => "/opds/v2",
        };
        format!(
            "{}{}",
            self.context,
            servlet_uri::encode_path(&format!("{root}/{path}"))
        )
    }

    /// `getExtraLinkProperties`.
    fn properties(&self) -> Option<WpLinkProperties> {
        (self.generator == Generator::Opds).then(|| WpLinkProperties {
            authenticate: WpAuthenticate {
                href: self.root_url("auth"),
                media_type: OPDS_AUTHENTICATION_JSON,
            },
        })
    }

    /// `BookDto.toWPLinkDtos`.
    fn links(&self) -> Vec<WpLink> {
        let book = self.book;
        let profile = self.profile;
        let mut links = vec![WpLink {
            rel: Some("self"),
            href: Some(self.url("manifest")),
            media_type: Some(media_type(profile).to_owned()),
            properties: self.properties(),
            ..WpLink::default()
        }];
        let divina = match profile {
            Some(MediaProfile::Pdf) => true,
            Some(MediaProfile::Epub) => book.media.epub_divina_compatible,
            Some(MediaProfile::Divina) | None => false,
        };
        if divina {
            links.push(WpLink {
                href: Some(self.url("manifest/divina")),
                media_type: Some(DIVINA_JSON.to_owned()),
                properties: self.properties(),
                ..WpLink::default()
            });
        }
        links.push(WpLink {
            rel: Some(ACQUISITION),
            href: Some(self.url("file")),
            media_type: book.media.media_type.as_deref().map(export_type),
            properties: self.properties(),
            ..WpLink::default()
        });
        if self.generator == Generator::Opds {
            links.push(WpLink {
                rel: Some(PROGRESSION),
                href: Some(self.url("progression")),
                media_type: Some(PROGRESSION_JSON.to_owned()),
                properties: self.properties(),
                ..WpLink::default()
            });
        }
        links
    }

    /// `buildThumbnailLinkDtos`.
    fn thumbnail(&self) -> WpLink {
        WpLink {
            href: Some(self.url("thumbnail")),
            media_type: Some(THUMBNAIL_TYPE.to_owned()),
            properties: self.properties(),
            ..WpLink::default()
        }
    }

    /// `getBookSeriesLink`.
    fn series_links(&self) -> Vec<WpLink> {
        match self.generator {
            Generator::WebPub => Vec::new(),
            Generator::Opds => vec![WpLink {
                href: Some(self.root_url(&format!("series/{}", self.book.book.series_id))),
                media_type: Some(OPDS_JSON.to_owned()),
                ..WpLink::default()
            }],
        }
    }

    /// A DiViNa page, with a JPEG alternate when its format is not recommended but convertible.
    fn page_link(&self, number: usize, page: &BookPage) -> WpLink {
        let href = format!(
            "{}?contentNegotiation=false",
            self.url(&format!("pages/{number}"))
        );
        let (width, height) = page
            .dimension
            .map_or((None, None), |size| (Some(size.width), Some(size.height)));
        let convertible = !RECOMMENDED_IMAGE_TYPES.contains(&page.media_type.as_str())
            && can_convert_to_jpeg(&page.media_type);
        let alternate = if convertible {
            vec![WpLink {
                href: Some(format!("{href}&convert=jpeg")),
                media_type: Some("image/jpeg".to_owned()),
                width,
                height,
                ..WpLink::default()
            }]
        } else {
            Vec::new()
        };
        WpLink {
            href: Some(href),
            media_type: Some(page.media_type.clone()),
            width,
            height,
            alternate,
            ..WpLink::default()
        }
    }

    /// `EpubTocEntry.toWPLinkDto`: the fragment after the last `#` is appended unencoded.
    fn toc_link(&self, entry: &EpubTocEntry) -> WpLink {
        let href = entry.href.as_deref().map(|href| {
            let (path, fragment) = href.rsplit_once('#').unwrap_or((href, ""));
            let url = self.url(&format!("resource/{path}"));
            if fragment.is_empty() {
                url
            } else {
                format!("{url}#{fragment}")
            }
        });
        WpLink {
            title: Some(entry.title.clone()),
            href,
            children: entry
                .children
                .iter()
                .map(|child| self.toc_link(child))
                .collect(),
            ..WpLink::default()
        }
    }
}

/// `MediaType.exportType`: comic book types for ZIP and RAR, the media type otherwise.
fn export_type(media_type: &str) -> String {
    match media_type {
        "application/zip" => "application/vnd.comicbook+zip",
        "application/x-rar-compressed"
        | "application/x-rar-compressed; version=4"
        | "application/x-rar-compressed; version=5" => "application/vnd.comicbook-rar",
        other => other,
    }
    .to_owned()
}
