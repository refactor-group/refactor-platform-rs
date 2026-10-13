//! Query parameters for the transcript suggestion endpoints.

use serde::Deserialize;
use utoipa::IntoParams;

/// Query parameters for listing a coaching session's suggested Actions.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct IndexParams {
    /// Kind of suggestion to list; only `action` (the default) is supported.
    pub(crate) target: Option<String>,
    /// `pending` (the default) or `dismissed`.
    pub(crate) status: Option<String>,
}

/// Query parameters for reading or starting a scan of a coaching session's transcript.
#[derive(Debug, Deserialize, IntoParams)]
#[into_params(parameter_in = Query)]
pub(crate) struct ScanParams {
    /// Kind of suggestion to scan for; only `action` (the default) is supported.
    pub(crate) target: Option<String>,
}
