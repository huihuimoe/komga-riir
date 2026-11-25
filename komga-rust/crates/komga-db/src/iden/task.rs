//! The tasks database.

use sea_query::Iden;

/// Tasks database `TASK` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "TASK"]
pub enum TaskTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "PRIORITY"]
    Priority,
    #[iden = "GROUP_ID"]
    GroupId,
    #[iden = "CLASS"]
    Class,
    #[iden = "SIMPLE_TYPE"]
    SimpleType,
    #[iden = "PAYLOAD"]
    Payload,
    #[iden = "OWNER"]
    Owner,
    #[iden = "CREATED_DATE"]
    CreatedDate,
    #[iden = "LAST_MODIFIED_DATE"]
    LastModifiedDate,
}

/// Main database `ERA_TASK_HANDOVER`: tasks committed with the main database changes that call for
/// them, not yet moved to the tasks database.
#[derive(Iden, Clone, Copy)]
#[iden = "ERA_TASK_HANDOVER"]
pub enum TaskHandoverTable {
    Table,
    #[iden = "SEQ"]
    Seq,
    #[iden = "ID"]
    Id,
    #[iden = "PRIORITY"]
    Priority,
    #[iden = "GROUP_ID"]
    GroupId,
    #[iden = "CLASS"]
    Class,
    #[iden = "SIMPLE_TYPE"]
    SimpleType,
    #[iden = "PAYLOAD"]
    Payload,
}
