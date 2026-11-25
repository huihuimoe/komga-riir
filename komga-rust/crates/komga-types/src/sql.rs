//! SQLite column and sea-query value support for the domain types.

/// Lets string newtypes be read from SQLite `TEXT` columns and passed to sea-query as values.
macro_rules! sql_text {
    ($($type:ty),+ $(,)?) => {$(
        impl sqlx::Type<sqlx::Sqlite> for $type {
            fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
                <String as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for $type {
            fn decode(
                value: sqlx::sqlite::SqliteValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                <String as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value).map(Self::from)
            }
        }

        impl From<$type> for sea_query::Value {
            fn from(value: $type) -> Self {
                Self::String(Some(value.0))
            }
        }

        impl From<&$type> for sea_query::Value {
            fn from(value: &$type) -> Self {
                Self::String(Some(value.0.clone()))
            }
        }
    )+};
}
pub(crate) use sql_text;

/// Lets enums with a stored name be read from SQLite `TEXT` columns and passed to sea-query as
/// values; `$name` is the method returning the stored name.
macro_rules! sql_enum {
    ($($type:ty => $name:ident),+ $(,)?) => {$(
        impl sqlx::Type<sqlx::Sqlite> for $type {
            fn type_info() -> sqlx::sqlite::SqliteTypeInfo {
                <str as sqlx::Type<sqlx::Sqlite>>::type_info()
            }
        }

        impl<'r> sqlx::Decode<'r, sqlx::Sqlite> for $type {
            fn decode(
                value: sqlx::sqlite::SqliteValueRef<'r>,
            ) -> Result<Self, sqlx::error::BoxDynError> {
                let name = <&str as sqlx::Decode<'r, sqlx::Sqlite>>::decode(value)?;
                name.parse()
                    .map_err(|_| format!("Unknown {} {name}", stringify!($type)).into())
            }
        }

        impl From<$type> for sea_query::Value {
            fn from(value: $type) -> Self {
                Self::String(Some(value.$name().to_owned()))
            }
        }

        impl sea_query::Nullable for $type {
            fn null() -> sea_query::Value {
                sea_query::Value::String(None)
            }
        }
    )+};
}
pub(crate) use sql_enum;
