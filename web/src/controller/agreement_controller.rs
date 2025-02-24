use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::params::agreement::IndexParams;
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::agreement as AgreementApi;
use entity::{agreements::Model, Id};
use serde_json::json;
use service::config::ApiVersion;

use log::*;

/// POST create a new Agreement
#[utoipa::path(
    post,
    path = "/agreements",
    params(ApiVersion),
    request_body = entity::agreements::Model,
    responses(
        (status = 201, description = "Successfully Created a New Agreement", body = [entity::agreements::Model]),
        (status= 422, description = "Unprocessable Entity"),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]

pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Json(agreement_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("POST Create a New Agreement from: {:?}", agreement_model);

    let agreement = AgreementApi::create(app_state.db_conn_ref(), agreement_model, user.id).await?;

    debug!("New Agreement: {:?}", agreement);

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        agreement,
    )))
}

/// GET a particular Agreement specified by its id.
#[utoipa::path(
    get,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = String, Path, description = "Agreement id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a specific Agreement by its id", body = [entity::notes::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Agreement not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET Agreement by id: {}", id);

    let note: Option<Model> = AgreementApi::find_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), note)))
}

#[utoipa::path(
    put,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = Id, Path, description = "Id of agreement to update"),
    ),
    request_body = entity::agreements::Model,
    responses(
        (status = 200, description = "Successfully Updated Agreement", body = [entity::agreements::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(agreement_model): Json<Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("PUT Update Agreement with id: {}", id);

    let agreement = AgreementApi::update(app_state.db_conn_ref(), id, agreement_model).await?;

    debug!("Updated Agreement: {:?}", agreement);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), agreement)))
}

#[utoipa::path(
    get,
    path = "/agreements",
    params(
        ApiVersion,
        ("coaching_session_id" = Id, Query, description = "Filter by coaching_session_id")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Agreements", body = [entity::agreements::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
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
    info!("Params: {:?}", params);
    let agreements = AgreementApi::find_by(app_state.db_conn_ref(), params).await?;
    Ok(Json(ApiResponse::new(StatusCode::OK.into(), agreements)))
}

/// DELETE an Agreement specified by its primary key.
#[utoipa::path(
    delete,
    path = "/agreements/{id}",
    params(
        ApiVersion,
        ("id" = i32, Path, description = "Agreement id to delete")
    ),
    responses(
        (status = 200, description = "Successfully deleted a certain Agreement by its id", body = [i32]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Agreement not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn delete(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("DELETE Agreement by id: {}", id);

    AgreementApi::delete_by_id(app_state.db_conn_ref(), id).await?;
    Ok(Json(json!({"id": id})))
}
