//! The komga-rust table (`era` migrations) of file operations under way.

use sea_query::Iden;

/// `ERA_FILE_OPERATION`: files being written next to or over library files.
#[derive(Iden, Clone, Copy)]
#[iden = "ERA_FILE_OPERATION"]
pub enum FileOperationTable {
    Table,
    #[iden = "ID"]
    Id,
    #[iden = "SOURCE_URL"]
    SourceUrl,
    #[iden = "SOURCE_SIZE"]
    SourceSize,
    #[iden = "SOURCE_MODIFIED"]
    SourceModified,
    #[iden = "TARGET_URL"]
    TargetUrl,
    #[iden = "REPLACED_URL"]
    ReplacedUrl,
    #[iden = "OUTPUT_SIZE"]
    OutputSize,
    #[iden = "REMOVE_SOURCE"]
    RemoveSource,
    #[iden = "OUTPUT_MODIFIED"]
    OutputModified,
    #[iden = "COMMITTED"]
    Committed,
}
