//! Request errors and their Spring-compatible responses.

mod api_error;
mod response;
mod service_error;

pub(crate) use api_error::{ApiError, Violation};
pub(crate) use response::negotiate;
