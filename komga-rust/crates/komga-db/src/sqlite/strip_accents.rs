//! The `UDF_STRIP_ACCENTS` SQL function of Kotlin `SqliteUdfDataSource`.
//!
//! SQLx registers collations but not functions, so this goes through the SQLite C API on the
//! connection SQLx opened.

use std::ffi::c_int;
use std::{ptr, slice, str};

use icu_normalizer::DecomposingNormalizerBorrowed;
use libsqlite3_sys as ffi;
use sea_query::Expr;
use sqlx::SqliteConnection;

/// `UDF_STRIP_ACCENTS(text)`.
pub(crate) fn call_sql(text: Expr) -> Expr {
    Expr::cust_with_expr("UDF_STRIP_ACCENTS(?)", text)
}

/// Apache Commons `StringUtils.stripAccents`: compatibility decomposition without combining
/// diacritical marks (U+0300–U+036F), and the stroked letters that do not decompose.
pub(crate) fn strip_accents(text: &str) -> String {
    DecomposingNormalizerBorrowed::new_nfkd()
        .normalize(text)
        .chars()
        .filter(|character| !('\u{300}'..='\u{36f}').contains(character))
        .map(|character| match character {
            'Ł' => 'L',
            'ł' => 'l',
            'Đ' => 'D',
            'đ' => 'd',
            other => other,
        })
        .collect()
}

/// Registers the function on a newly opened connection.
pub(crate) async fn register(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut handle = connection.lock_handle().await?;
    // SAFETY: The handle is a live connection that the lock keeps SQLx's worker thread off for
    // the duration of the call; the name is NUL-terminated and the callback has the signature of
    // a one-argument scalar function and keeps no user data.
    let code = unsafe {
        ffi::sqlite3_create_function_v2(
            handle.as_raw_handle().as_ptr(),
            c"UDF_STRIP_ACCENTS".as_ptr(),
            1,
            ffi::SQLITE_UTF8 | ffi::SQLITE_DETERMINISTIC,
            ptr::null_mut(),
            Some(call),
            None,
            None,
            None,
        )
    };
    if code == ffi::SQLITE_OK {
        Ok(())
    } else {
        Err(sqlx::Error::Configuration(
            format!("Registering UDF_STRIP_ACCENTS failed with SQLite code {code}").into(),
        ))
    }
}

/// NULL stays NULL.
unsafe extern "C" fn call(
    context: *mut ffi::sqlite3_context,
    _count: c_int,
    values: *mut *mut ffi::sqlite3_value,
) {
    // SAFETY: SQLite passes the single registered argument, whose text stays valid until the
    // function returns; the result is copied by SQLite (`SQLITE_TRANSIENT`).
    unsafe {
        let value = *values;
        let text = ffi::sqlite3_value_text(value);
        if text.is_null() {
            ffi::sqlite3_result_null(context);
            return;
        }
        let length = usize::try_from(ffi::sqlite3_value_bytes(value)).unwrap_or(0);
        let Ok(text) = str::from_utf8(slice::from_raw_parts(text, length)) else {
            ffi::sqlite3_result_error(context, c"Text is not UTF-8".as_ptr(), -1);
            return;
        };
        let stripped = strip_accents(text);
        let Ok(length) = c_int::try_from(stripped.len()) else {
            ffi::sqlite3_result_error_toobig(context);
            return;
        };
        ffi::sqlite3_result_text(
            context,
            stripped.as_ptr().cast(),
            length,
            ffi::SQLITE_TRANSIENT(),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::strip_accents;

    /// Outputs of commons-lang3 3.21.0.
    #[test]
    fn strips_like_commons_lang() {
        for (input, expected) in [
            ("Café", "Cafe"),
            ("Łódź", "Lodz"),
            ("Đđ", "Dd"),
            ("ﬁ①Ａ²", "fi1A2"),
            ("x\u{301}\u{302}y", "xy"),
            ("Øreæßœ", "Øreæßœ"),
            ("a\u{1dc0}", "a\u{1dc0}"),
        ] {
            assert_eq!(strip_accents(input), expected, "Stripping {input:?}");
        }
    }
}
