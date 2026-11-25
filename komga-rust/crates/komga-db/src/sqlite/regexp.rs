//! The `REGEXP` SQL function of Kotlin `SqliteUdfDataSource`, matching a [`TitleRegex`].
//!
//! It is registered through the SQLite C API like [`super::strip_accents`].

use std::ffi::{c_int, c_void};
use std::{ptr, slice, str};

use fancy_regex::Regex;
use komga_types::TitleRegex;
use libsqlite3_sys as ffi;
use sea_query::Expr;
use sqlx::SqliteConnection;

/// `REGEXP(pattern, text)`: whether `text` contains a match of `regex`.
pub(crate) fn call_sql(regex: &TitleRegex, text: Expr) -> Expr {
    Expr::cust_with_exprs("REGEXP(?, ?)", [Expr::val(regex.pattern()), text])
}

/// Registers the function on a newly opened connection.
pub(crate) async fn register(connection: &mut SqliteConnection) -> Result<(), sqlx::Error> {
    let mut handle = connection.lock_handle().await?;
    // SAFETY: The handle is a live connection that the lock keeps SQLx's worker thread off for
    // the duration of the call; the name is NUL-terminated and the callback has the signature of
    // a two-argument scalar function and keeps no user data.
    let code = unsafe {
        ffi::sqlite3_create_function_v2(
            handle.as_raw_handle().as_ptr(),
            c"REGEXP".as_ptr(),
            2,
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
            format!("Registering REGEXP failed with SQLite code {code}").into(),
        ))
    }
}

/// NULL reads as empty text, as in Kotlin. The compiled pattern is kept as auxiliary data of the
/// statement, so that it compiles once per query rather than once per row.
unsafe extern "C" fn call(
    context: *mut ffi::sqlite3_context,
    _count: c_int,
    values: *mut *mut ffi::sqlite3_value,
) {
    // SAFETY: SQLite passes the two registered arguments, whose texts stay valid until the
    // function returns. Auxiliary data 0 is only ever set here, to a boxed `Regex` that SQLite
    // frees with `drop_regex`; it is read before being replaced.
    unsafe {
        let Some(text) = value_text(*values.add(1)) else {
            ffi::sqlite3_result_error(context, c"Text is not UTF-8".as_ptr(), -1);
            return;
        };
        let cached = ffi::sqlite3_get_auxdata(context, 0).cast::<Regex>();
        if let Some(regex) = cached.as_ref() {
            set_match(context, regex, text);
            return;
        }
        let Some(pattern) = value_text(*values) else {
            ffi::sqlite3_result_error(context, c"Pattern is not UTF-8".as_ptr(), -1);
            return;
        };
        let Ok(regex) = TitleRegex::compile(pattern) else {
            ffi::sqlite3_result_error(context, c"Invalid regular expression".as_ptr(), -1);
            return;
        };
        set_match(context, &regex, text);
        let boxed = Box::into_raw(Box::new(regex));
        ffi::sqlite3_set_auxdata(context, 0, boxed.cast(), Some(drop_regex));
    }
}

/// The UTF-8 text of `value`, empty for NULL.
///
/// # Safety
/// `value` must be a protected value of the running function call; the text lives until it
/// returns.
unsafe fn value_text<'call>(value: *mut ffi::sqlite3_value) -> Option<&'call str> {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        let text = ffi::sqlite3_value_text(value);
        if text.is_null() {
            return Some("");
        }
        let length = usize::try_from(ffi::sqlite3_value_bytes(value)).unwrap_or(0);
        str::from_utf8(slice::from_raw_parts(text, length)).ok()
    }
}

/// Sets 1 or 0; a match that exceeds the backtracking limit fails the query.
///
/// # Safety
/// `context` must be the context of the running function call.
unsafe fn set_match(context: *mut ffi::sqlite3_context, regex: &Regex, text: &str) {
    // SAFETY: Guaranteed by the caller.
    unsafe {
        match regex.is_match(text) {
            Ok(matched) => ffi::sqlite3_result_int(context, c_int::from(matched)),
            Err(_) => ffi::sqlite3_result_error(
                context,
                c"Regular expression exceeded its backtracking limit".as_ptr(),
                -1,
            ),
        }
    }
}

/// # Safety
/// `regex` must come from `Box::into_raw` of a `Regex` in [`call`].
unsafe extern "C" fn drop_regex(regex: *mut c_void) {
    // SAFETY: Guaranteed by the caller; SQLite calls this once per stored pointer.
    drop(unsafe { Box::from_raw(regex.cast::<Regex>()) });
}
