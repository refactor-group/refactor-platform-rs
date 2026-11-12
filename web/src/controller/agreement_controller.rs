use crate::controller::ApiResponse;
use crate::extractors::{
    agreement_access::AgreementAccess, authenticated_user::AuthenticatedUser,
    coaching_session_access::authorize_session, compare_api_version::CompareApiVersion,
};
use crate::params::agreement::{IndexParams, SortField};
use crate::params::WithSortDefaults;
use crate::{AppState, Error};
use axum::extract::{Query, State};
use axum::http::StatusCode;
use axum::response::{IntoResponse, Response};
use axum::Json;
use domain::{agreement as AgreementApi, agreements::Model};

use serde_json::json;
use service::config::ApiVersion;

use log::*;

/// POST create a new Agreement
#[utoipa::path(
    post,
    path = "/agreements",
    params(ApiVersion),
    request_body = domain::agreements::Model,
    responses(
        (status = 201, description = "Successfully Created a New Agreement", body = domain::agreements::Model),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Coaching session not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]

pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    State(app_state): State<AppState>,
    Json(agreement_model): Json<Model>,
) -> Result<Response, Error> {
    debug!("POST Create a New Agreement from: {agreement_model:?}");

    if let Err(rejection) =
        authorize_session(&app_state, &user, agreement_model.coaching_session_id).await
    {
        return Ok(rejection.into_response());
    }

    let agreement = AgreementApi::create(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        agreement_model,
        user.id,
    )
    .await?;

    debug!("New Agreement: {agreement:?}");

    Ok(Json(ApiResponse::new(StatusCode::CREATED.into(), agreement)).into_response())
}

/// GET a particular Agreement specified by its id.
#[utoipa::path(
    get,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Agreement id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Agreement by its id", body = domain::agreements::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Agreement not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AgreementAccess { agreement }: AgreementAccess,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Agreement by id: {}", agreement.id);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), agreement)))
}

#[utoipa::path(
    put,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Id of agreement to update"),
    ),
    request_body = domain::agreements::Model,
    responses(
        (status = 200, description = "Successfully Updated Agreement", body = domain::agreements::Model),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Agreement not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    AgreementAccess { agreement }: AgreementAccess,
    State(app_state): State<AppState>,
    Json(agreement_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    let id = agreement.id;
    debug!("PUT Update Agreement with id: {id}");

    let agreement = AgreementApi::update(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
        agreement_model,
    )
    .await?;

    debug!("Updated Agreement: {agreement:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), agreement)))
}

#[utoipa::path(
    get,
    path = "/agreements",
    params(
        ApiVersion,
        ("coaching_session_id" = Uuid, Query, description = "Filter by coaching_session_id"),
        ("sort_by" = Option<crate::params::agreement::SortField>, Query, description = "Sort by field. Valid values: 'body', 'created_at', 'updated_at'. Must be provided with sort_order.", example = "body"),
        ("sort_order" = Option<crate::params::sort::SortOrder>, Query, description = "Sort order. Valid values: 'asc' (ascending), 'desc' (descending). Must be provided with sort_by.", example = "desc")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Agreements", body = [domain::agreements::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(params): Query<IndexParams>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Agreements");
    debug!("Filter Params: {params:?}");

    // Apply default sorting parameters
    let mut params = params;
    IndexParams::apply_sort_defaults(&mut params.sort_by, &mut params.sort_order, SortField::Body);

    let agreements = AgreementApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Agreements: {agreements:?}");

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), agreements)))
}

/// DELETE an Agreement specified by its primary key.
#[utoipa::path(
    delete,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = Uuid, Path, description = "Agreement id to delete")
    ),
    responses(
        (status = 200, description = "Successfully deleted a certain Agreement by its id", body = serde_json::Value),
        (status = 401, description = "Unauthorized"),
        (status = 403, description = "Forbidden"),
        (status = 404, description = "Agreement not found"),
        (status = 405, description = "Method not allowed"),
        (status = 503, description = "Service temporarily unavailable")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    AgreementAccess { agreement }: AgreementAccess,
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    let id = agreement.id;
    debug!("DELETE Agreement by id: {id}");

    AgreementApi::delete_by_id(
        app_state.db_conn_ref(),
        app_state.event_publisher.as_ref(),
        id,
    )
    .await?;
    Ok(Json(json!({"id": id})))
}

#[cfg(test)]
#[path = "agreement_controller_sqlite_tests.rs"]
mod sqlite_tests;
