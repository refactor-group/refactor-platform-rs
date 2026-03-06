//! HTTP client building with middleware.

mod client;
mod retry;

pub use client::{Builder, Client, Config};
pub use retry::RetryAfterPolicy;
