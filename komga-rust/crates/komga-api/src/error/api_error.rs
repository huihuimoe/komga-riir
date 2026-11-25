//! The HTTP error of a request, and its common cases.

use std::borrow::Cow;

use axum::http::StatusCode;
use serde::Serialize;

pub(crate) enum ApiError {
    Spring {
        status: StatusCode,
        message: String,
        path: String,
    },
    /// `ErrorHandlingControllerAdvice.onMethodArgumentNotValidException` and
    /// `onConstraintValidationException`: 400 with violations.
    Validation(Vec<Violation>),
    /// An RFC 9457 `ProblemDetail` with the request path as its instance, as
    /// `ErrorHandlingControllerAdvice.handleMaxUploadSizeExceededException` and the
    /// `ResponseEntityExceptionHandler` of `ApiV3ErrorHandlingControllerAdvice` reply.
    Problem {
        status: StatusCode,
        detail: String,
        path: String,
    },
}

/// One field failure in a baseline `ValidationErrorResponse`.
#[derive(Debug, Serialize)]
#[serde(rename_all = "camelCase")]
pub(crate) struct Violation {
    field_name: Cow<'static, str>,
    message: &'static str,
}

impl Violation {
    pub(crate) fn new(field_name: impl Into<Cow<'static, str>>, message: &'static str) -> Self {
        Self {
            field_name: field_name.into(),
            message,
        }
    }
}

impl ApiError {
    pub(crate) fn new(status: StatusCode, message: impl Into<String>, path: &str) -> Self {
        Self::Spring {
            status,
            message: message.into(),
            path: path.to_owned(),
        }
    }

    pub(crate) fn problem(status: StatusCode, detail: impl Into<String>, path: &str) -> Self {
        Self::Problem {
            status,
            detail: detail.into(),
            path: path.to_owned(),
        }
    }

    /// Spring Security's `AccessDeniedHandlerImpl` reply to any failed `@PreAuthorize` rule.
    pub(crate) fn forbidden(path: &str) -> Self {
        Self::new(StatusCode::FORBIDDEN, "Forbidden", path)
    }

    /// Logs the underlying error and returns a generic 500 response without leaking internals.
    pub(crate) fn internal_error(path: &str, error: impl std::fmt::Display) -> Self {
        tracing::error!(path, %error, "Internal server error");
        Self::new(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Internal server error",
            path,
        )
    }

    /// Fails the request when Bean Validation found any violation.
    pub(crate) fn check_violations(violations: Vec<Violation>) -> Result<(), Self> {
        if violations.is_empty() {
            Ok(())
        } else {
            Err(Self::Validation(violations))
        }
    }
}
