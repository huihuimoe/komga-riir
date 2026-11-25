use std::collections::BTreeMap;

use serde::Serialize;

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
pub(in crate::sse) struct TaskQueueSseDto {
    pub count: i64,
    pub count_by_type: BTreeMap<String, i64>,
}
