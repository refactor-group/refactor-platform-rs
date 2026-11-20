//! A job context for jobs tests.

use std::sync::Arc;

use sea_orm::DatabaseConnection;
use service::config::Config;

use crate::jobs::Context;

/// A job context over `db` with a plain test config.
pub(crate) fn job_context(db: DatabaseConnection) -> Context {
    job_context_with(db, Config::from_args(["test"]))
}

/// A job context over `db` with `config`.
pub(crate) fn job_context_with(db: DatabaseConnection, config: Config) -> Context {
    Context {
        db: Arc::new(db),
        config,
    }
}
