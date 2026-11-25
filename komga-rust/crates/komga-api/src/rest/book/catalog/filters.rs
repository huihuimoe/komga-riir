//! Sort mapping, query filters, and page formatting for book catalog endpoints.

use axum::Json;
use komga_types::{
    BookCondition, BookDetails, BookSort, Equality, EqualityNullable, PageRequest, PageResult,
    SortOrder,
};

use crate::mvc::pageable::{SpringPage, keyed};
use crate::mvc::request_params::RequestParams;
use crate::rest::dto::BookDto;

/// The requested orders by known properties, but read list numbers, which order books only when
/// a condition selects a read list, and relevance, which orders only full-text searches.
pub(super) fn known_sort(params: &RequestParams) -> Vec<SortOrder<BookSort>> {
    let mut sort = keyed(params, book_sort);
    sort.retain(|order| ![BookSort::ReadListNumber, BookSort::Relevance].contains(&order.key));
    sort
}

/// `BookDtoDao.sorts`.
pub(super) fn book_sort(property: &str) -> Option<BookSort> {
    Some(match property {
        "name" => BookSort::Name,
        "series" => BookSort::SeriesTitleSort,
        "created" | "createdDate" => BookSort::CreatedDate,
        "lastModified" | "lastModifiedDate" => BookSort::LastModifiedDate,
        "fileSize" | "size" => BookSort::FileSize,
        "fileHash" => BookSort::FileHash,
        "url" => BookSort::Url,
        "media.status" => BookSort::MediaStatus,
        "media.comment" => BookSort::MediaComment,
        "media.mediaType" => BookSort::MediaType,
        "media.pagesCount" => BookSort::MediaPageCount,
        "metadata.title" => BookSort::Title,
        "metadata.numberSort" => BookSort::NumberSort,
        "metadata.releaseDate" => BookSort::ReleaseDate,
        "readProgress.lastModified" => BookSort::ReadProgressLastModified,
        "readProgress.readDate" => BookSort::ReadDate,
        "readList.number" => BookSort::ReadListNumber,
        "relevance" => BookSort::Relevance,
        _ => return None,
    })
}

/// Only admins see the paths of the files.
pub(super) fn book_page(
    result: PageResult<BookDetails>,
    page: PageRequest,
    sorted: bool,
    admin: bool,
) -> Json<SpringPage<BookDto>> {
    let result = PageResult {
        items: result
            .items
            .into_iter()
            .map(|book| BookDto::new(book, admin))
            .collect(),
        total_elements: result.total_elements,
    };
    Json(SpringPage::new(result, page, sorted))
}

#[derive(Debug, Clone, Copy)]
pub(super) enum ListFilter {
    Library,
    ReadStatus,
    MediaStatus,
    Tag,
    Author,
}

pub(super) fn any_of_filters(
    params: &RequestParams,
    filters: &[ListFilter],
) -> Result<Vec<BookCondition>, String> {
    fn any_of<T>(
        values: Option<Vec<T>>,
        condition: fn(T) -> BookCondition,
    ) -> Option<BookCondition> {
        values
            .filter(|values| !values.is_empty())
            .map(|values| BookCondition::AnyOf(values.into_iter().map(condition).collect()))
    }
    let mut conditions = Vec::new();
    for filter in filters {
        let condition = match filter {
            ListFilter::Library => any_of(params.list("library_id")?, |id| {
                BookCondition::LibraryId(Equality::Is { value: id })
            }),
            ListFilter::ReadStatus => any_of(params.list("read_status")?, |status| {
                BookCondition::ReadStatus(Equality::Is { value: status })
            }),
            ListFilter::MediaStatus => any_of(params.list("media_status")?, |status| {
                BookCondition::MediaStatus(Equality::Is { value: status })
            }),
            ListFilter::Tag => any_of(params.list("tag")?, |tag| {
                BookCondition::Tag(EqualityNullable::Is { value: tag })
            }),
            ListFilter::Author => any_of(Some(params.authors()), |author| {
                BookCondition::Author(Equality::Is { value: author })
            }),
        };
        conditions.extend(condition);
    }
    Ok(conditions)
}
