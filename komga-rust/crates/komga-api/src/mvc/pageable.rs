//! Spring Data `Pageable` resolution and `PageImpl` JSON representation.

use axum::Json;
use komga_types::{PageRequest, PageResult, SortDirection, SortOrder};
use serde::Serialize;

use crate::mvc::request_params::RequestParams;

const DEFAULT_PAGE_SIZE: u32 = 20;
const MAX_PAGE_SIZE: u32 = 2000;
const MAX_PAGE_NUMBER: u32 = i32::MAX.unsigned_abs();

/// `PageableHandlerMethodArgumentResolver` defaults, then Komga's `unpackPageParams`.
///
/// Unparsable numbers become 0 and negatives are clamped to 0; a size below 1 falls back to 20
/// and above 2000 is capped.
pub(crate) fn page_request(params: &RequestParams, unpaged: bool) -> PageRequest {
    if unpaged {
        return PageRequest::Unpaged;
    }
    let number = bounded(params.first("page"), MAX_PAGE_NUMBER).unwrap_or(0);
    let size = match bounded(params.first("size"), MAX_PAGE_SIZE) {
        Some(0) | None => DEFAULT_PAGE_SIZE,
        Some(size) => size,
    };
    PageRequest::Paged { number, size }
}

/// `SortHandlerMethodArgumentResolver`: each `sort` value is `property,...[,asc|desc][,ignorecase]`.
///
/// Queries ignore case-insensitivity, so it is dropped.
pub(crate) fn sort_orders(params: &RequestParams) -> Vec<SortOrder<String>> {
    let values = params.all("sort");
    if let [single] = values.as_slice()
        && single.trim().is_empty()
    {
        return Vec::new();
    }
    let mut orders = Vec::new();
    for value in values {
        let mut elements: Vec<&str> = value
            .split(',')
            .filter(|element| !element.replace('.', "").trim().is_empty())
            .collect();
        if elements
            .last()
            .is_some_and(|last| last.eq_ignore_ascii_case("ignorecase"))
        {
            elements.pop();
        }
        let direction = match elements.last().map(|last| last.to_uppercase()).as_deref() {
            Some("ASC") => Some(SortDirection::Asc),
            Some("DESC") => Some(SortDirection::Desc),
            _ => None,
        };
        if direction.is_some() {
            elements.pop();
        }
        orders.extend(elements.into_iter().map(|property| SortOrder {
            key: property.to_owned(),
            direction: direction.unwrap_or(SortDirection::Asc),
        }));
    }
    orders
}

/// Spring `parseAndApplyBoundaries`; `None` for a missing or blank parameter.
fn bounded(value: Option<&str>, upper: u32) -> Option<u32> {
    let value = value.filter(|value| !value.trim().is_empty())?;
    let parsed = value.parse::<i32>().unwrap_or(0);
    Some(u32::try_from(parsed).unwrap_or(0).min(upper))
}

/// Serialized form of Spring Data `PageImpl`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct SpringPage<T> {
    content: Vec<T>,
    pageable: PageableDto,
    last: bool,
    total_pages: u64,
    total_elements: u64,
    size: u32,
    number: u32,
    sort: SortDto,
    first: bool,
    number_of_elements: usize,
    empty: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
#[serde(rename_all = "camelCase")]
struct PageableDto {
    page_number: u32,
    page_size: u32,
    sort: SortDto,
    offset: u64,
    paged: bool,
    unpaged: bool,
}

#[derive(Debug, Clone, Copy, Serialize)]
struct SortDto {
    empty: bool,
    sorted: bool,
    unsorted: bool,
}

impl<T> SpringPage<T> {
    /// Komga `buildPage`: an unpaged result is shown as page 0 sized `max(total, 20)`.
    ///
    /// `sorted` reflects the fixed sort a query reports, independent of request parameters.
    pub(crate) fn new(result: PageResult<T>, request: PageRequest, sorted: bool) -> Self {
        let (number, size) = match request {
            PageRequest::Paged { number, size } => (number, size),
            PageRequest::Unpaged => (
                0,
                u32::try_from(result.total_elements)
                    .unwrap_or(u32::MAX)
                    .max(DEFAULT_PAGE_SIZE),
            ),
        };
        let offset = u64::from(number) * u64::from(size);
        let content = result.items;
        // `PageImpl` derives the total from the last page's content when the count falls short.
        let total_elements =
            if !content.is_empty() && offset + u64::from(size) > result.total_elements {
                offset + content.len() as u64
            } else {
                result.total_elements
            };
        let total_pages = total_elements.div_ceil(u64::from(size));
        let sort = SortDto {
            empty: !sorted,
            sorted,
            unsorted: !sorted,
        };
        Self {
            pageable: PageableDto {
                page_number: number,
                page_size: size,
                sort,
                offset,
                paged: true,
                unpaged: false,
            },
            last: u64::from(number) + 1 >= total_pages,
            total_pages,
            total_elements,
            size,
            number,
            sort,
            first: number == 0,
            number_of_elements: content.len(),
            empty: content.is_empty(),
            content,
        }
    }
}

/// The requested orders whose property the query knows; `toOrderBy` drops the others.
pub(crate) fn keyed<K>(params: &RequestParams, key: fn(&str) -> Option<K>) -> Vec<SortOrder<K>> {
    sort_orders(params)
        .into_iter()
        .filter_map(|order| {
            key(&order.key).map(|key| SortOrder {
                key,
                direction: order.direction,
            })
        })
        .collect()
}

/// The known requested orders, of which `relevance` only with a full-text search; without any
/// requested, `relevance` with a full-text search and `unsorted` without (the controllers'
/// `Sort.by("relevance")` defaults).
pub(crate) fn full_text_sort<K: Copy + PartialEq>(
    params: &RequestParams,
    key: fn(&str) -> Option<K>,
    relevance: K,
    full_text: bool,
    unsorted: &[SortOrder<K>],
) -> Vec<SortOrder<K>> {
    if !sort_orders(params).is_empty() {
        let mut sort = keyed(params, key);
        sort.retain(|order| order.key != relevance || full_text);
        return sort;
    }
    if full_text {
        return vec![SortOrder {
            key: relevance,
            direction: SortDirection::Asc,
        }];
    }
    unsorted.to_vec()
}

/// The page reports the requested sort only when some order applied.
pub(crate) fn spring_page<T, D: From<T>, K>(
    result: PageResult<T>,
    page: PageRequest,
    sort: &[SortOrder<K>],
) -> Json<SpringPage<D>> {
    let result = PageResult {
        items: result.items.into_iter().map(D::from).collect(),
        total_elements: result.total_elements,
    };
    Json(SpringPage::new(result, page, !sort.is_empty()))
}

#[cfg(test)]
mod tests {
    use komga_types::{SortDirection, SortOrder};

    use super::sort_orders;
    use crate::mvc::request_params::RequestParams;

    fn orders(query: &str) -> Vec<(String, SortDirection)> {
        sort_orders(&RequestParams::parse(Some(query)))
            .into_iter()
            .map(|SortOrder { key, direction }| (key, direction))
            .collect()
    }

    #[test]
    fn sort_values_parse_like_spring() {
        use SortDirection::{Asc, Desc};
        let pairs = |expected: &[(&str, SortDirection)]| -> Vec<(String, SortDirection)> {
            expected
                .iter()
                .map(|(key, direction)| ((*key).to_owned(), *direction))
                .collect()
        };
        assert_eq!(orders("sort=a"), pairs(&[("a", Asc)]));
        assert_eq!(orders("sort=a,b,DeSc"), pairs(&[("a", Desc), ("b", Desc)]));
        assert_eq!(
            orders("sort=a,desc&sort=b"),
            pairs(&[("a", Desc), ("b", Asc)])
        );
        assert_eq!(orders("sort=a,asc,IgnoreCase"), pairs(&[("a", Asc)]));
        assert_eq!(orders("sort=a,,..,desc"), pairs(&[("a", Desc)]));
        assert_eq!(
            orders("sort=desc"),
            pairs(&[]),
            "a direction alone has no property"
        );
        assert_eq!(orders("sort=%20"), pairs(&[]));
        assert_eq!(orders("page=1"), pairs(&[]));
    }
}
