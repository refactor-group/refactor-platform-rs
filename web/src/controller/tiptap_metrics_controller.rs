//! Admin endpoints for TipTap document metrics.
//!
//! Three GETs under `/admin/tiptap/metrics/*`, each gated by the `SuperAdminAccess`
//! extractor.

use axum::extract::State;
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::tiptap_metrics as TiptapMetricsApi;
use service::config::ApiVersion;

use crate::controller::ApiResponse;
use crate::extractors::{
    compare_api_version::CompareApiVersion, super_admin_access::SuperAdminAccess,
};
use crate::{AppState, Error};

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
    _access: SuperAdminAccess,
    CompareApiVersion(_v): CompareApiVersion,
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
    _access: SuperAdminAccess,
    CompareApiVersion(_v): CompareApiVersion,
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
    _access: SuperAdminAccess,
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let report =
        TiptapMetricsApi::abandoned_documents(app_state.db_conn_ref(), &app_state.config).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), report)))
}

#[cfg(test)]
#[path = "tiptap_metrics_controller_sqlite_tests.rs"]
mod sqlite_tests;
