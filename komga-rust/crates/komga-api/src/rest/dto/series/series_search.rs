//! Kotlin `SeriesSearch`: the body of `POST /api/v1/series/list`, and the `GroupCountDto` of its
//! alphabetical groups.

use komga_types::{GroupCount, SeriesCondition};
use serde::{Deserialize, Serialize};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct SeriesSearchDto {
    pub condition: Option<SeriesCondition>,
    pub full_text_search: Option<String>,
}

#[derive(Debug, Serialize)]
pub struct GroupCountDto {
    pub group: String,
    pub count: u64,
}

impl From<GroupCount> for GroupCountDto {
    fn from(group: GroupCount) -> Self {
        Self {
            group: group.group,
            count: group.count,
        }
    }
}
