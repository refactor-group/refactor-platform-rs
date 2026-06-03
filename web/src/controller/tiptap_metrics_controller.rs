//! Admin endpoints for TipTap document metrics.
//!
//! Three GETs under `/admin/tiptap/metrics/*`, gated by SuperAdmin via the
//! `protect::tiptap_metrics::admin_only` middleware in the router.

use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::tiptap_metrics as TiptapMetricsApi;
use service::config::ApiVersion;

/// Get platform-wide TipTap totals
#[utoipa::path(
    get,
    path = "/admin/tiptap/metrics/totals",
    params(ApiVersion),
    responses(
        (status = 200, description = "Platform totals (total_documents, total_bytes, archived_documents)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - SuperAdmin only"),
        (status = 502, description = "TipTap upstream error"),
    ),
    security(("cookie_auth" = []))
)]

pub async fn platform_totals(
    // Extractors run in declaration order. ApiVersion comparison first - fail
    // fast on incompatible clients.
    CompareApiVersion(_v): CompareApiVersion,
    // Auth presence is confirmed by `require_auth` middleware in the router;
    // this extractor materializes the user. We don't read the user value
    // here - authorization happens via protect::tiptap_metrics::admin_only.
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    // `?` propagates `domain::Error` -> `web::Error` via the existing blanket
    // `From<E: Into<DomainError>> for Error` impl in web/src/error.rs
    let totals = TiptapMetricsApi::platform_totals(&app_state.config).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), totals)))
}

/// GET per-organization TipTap metrics.
#[utoipa::path(
    get,
    path = "/admin/tiptap/metrics/per-org",
    params(ApiVersion),
    responses(
        (status = 200, description = "Per-org metrics sorted by document_count desc"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - SuperAdmin only"),
        (status = 502, description = "TipTap upstream error"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn per_org_metrics(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let metrics =
        TiptapMetricsApi::per_org_metrics(app_state.db_conn_ref(), &app_state.config).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), metrics)))
}

/// GET abandoned (orphaned) TipTap documents.
#[utoipa::path(
    get,
    path = "/admin/tiptap/metrics/abandoned",
    params(ApiVersion),
    responses(
        (status = 200, description = "Abandoned-doc report (capped at 500; total_found preserved)"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden - SuperAdmin only"),
        (status = 502, description = "TipTap upstream error"),
    ),
    security(("cookie_auth" = []))
)]
pub async fn abandoned_documents(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let report =
        TiptapMetricsApi::abandoned_documents(app_state.db_conn_ref(), &app_state.config).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), report)))
}
