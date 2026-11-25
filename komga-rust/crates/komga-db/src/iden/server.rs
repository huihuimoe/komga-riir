//! Server-wide settings.

use sea_query::Iden;

/// `SERVER_SETTINGS` table column identifiers.
#[derive(Iden, Clone, Copy)]
#[iden = "SERVER_SETTINGS"]
pub enum ServerSettingsTable {
    Table,
    #[iden = "KEY"]
    Key,
    #[iden = "VALUE"]
    Value,
}
