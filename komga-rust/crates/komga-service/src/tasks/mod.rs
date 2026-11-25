//! Background task queuing, execution, failure classification, and queue inspection.

pub(crate) mod emitter;
mod failures;
mod handler;
mod service;

pub use handler::{TaskHandler, TaskHandlerConfig};
pub use service::{TaskError, TaskService};
