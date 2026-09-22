//! `GET /search` — unified keyword search across the caller's visible corpus.

use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::search as SearchApi;
use log::*;
use service::config::ApiVersion;

use crate::controller::ApiResponse;
use crate::extractors::{compare_api_version::CompareApiVersion, scope::Scope};
use crate::params::search::IndexParams;
use crate::{AppState, Error};

/// Search every entity type the caller may see, merged into one ranked list.
///
/// Authorization is embedded in the searchers' queries via the caller's
/// compiled `Scope` — results can never exceed what the caller could already
/// read, and every filter intersects that scope. Disallowed or not-yet-active
/// `types` values are silently dropped, never a 403.
#[utoipa::path(
    get,
    path = "/search",
    params(ApiVersion, IndexParams),
    responses(
        (status = 200, description = "Ranked search results", body = domain::search::Results),
        (status = 400, description = "Bad Request — structured body with a stable `error` discriminator: `query_too_short`, `unknown_type`, `contradictory_params`, `malformed_cursor`, `mode_unavailable`, `keyword_weight_out_of_range`, `invalid_timezone`"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 429, description = "Too many requests (per-IP throttle)"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    Scope(scope): Scope,
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET /search params: {params:?}");
    let spec = params.compile()?;
    let results = SearchApi::search(app_state.db_conn_ref(), &scope, spec).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), results)))
}
