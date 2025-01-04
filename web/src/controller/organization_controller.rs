use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, Query, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use entity::{organizations, Id};
use entity_api::organization as OrganizationApi;
use serde_json::json;
use service::config::ApiVersion;

use std::collections::HashMap;

use log::debug;

/// GET search Organizations by filtering.
#[utoipa::path(
    get,
    path = "/organizations",
    params(
        ApiVersion,
        ("user_id" = Option<String>, Query, description = "Filter by user_id")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all Organizations", body = [entity::organizations::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn index(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Query(mut params): Query<HashMap<String, String>>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Organizations");

    params.insert("user_id".to_string(), user.id.to_string());

    let organizations = OrganizationApi::find_by(app_state.db_conn_ref(), params).await?;

    debug!("Found Organizations: {:?}", organizations);

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), organizations)))
}

/// GET a particular Organization specified by its id.
#[utoipa::path(
    get,
    path = "/organizations/{id}",
    params(
        ApiVersion,
        ("id" = String, Path, description = "Organization id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain Organization by its id", body = [entity::organizations::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Organization not found"),
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
    debug!("GET Organization by id: {}", id);

    let organization: Option<organizations::Model> =
        OrganizationApi::find_by_id(app_state.db_conn_ref(), id).await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), organization)))
}

/// CREATE a new Organization.
#[utoipa::path(
    post,
    path = "/organizations",
    params(
        ApiVersion,
    ),
    request_body = entity::organizations::Model,
    responses(
        (status = 200, description = "Successfully created a new Organization", body = [entity::organizations::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
    )]
pub async fn create(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Json(organization_model): Json<organizations::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!("CREATE new Organization: {:?}", organization_model.name);

    let organization: organizations::Model =
        OrganizationApi::create(app_state.db_conn_ref(), organization_model).await?;

    debug!("Newly Created Organization: {:?}", &organization);

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        organization,
    )))
}

/// UPDATE a particular Organization specified by its primary key.
#[utoipa::path(
    put,
    path = "/organizations/{id}",
    params(
        ApiVersion,
        ("id" = i32, Path, description = "Organization id to update")
    ),
    request_body = entity::organizations::Model,
    responses(
        (status = 200, description = "Successfully updated a certain Organization by its id", body = [entity::organizations::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Organization not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn update(
    CompareApiVersion(_v): CompareApiVersion,
    State(app_state): State<AppState>,
    Path(id): Path<Id>,
    Json(organization_model): Json<organizations::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "UPDATE the entire Organization by id: {:?}, new name: {:?}",
        id, organization_model.name
    );

    let updated_organization: organizations::Model =
        OrganizationApi::update(app_state.db_conn_ref(), id, organization_model).await?;

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        updated_organization,
    )))
}

/// DELETE an Organization specified by its primary key.
#[utoipa::path(
    delete,
    path = "/organizations/{id}",
    params(
        ApiVersion,
        ("id" = i32, Path, description = "Organization id to delete")
    ),
    responses(
        (status = 200, description = "Successfully deleted a certain Organization by its id", body = [i32]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "Organization not found"),
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
    debug!("DELETE Organization by id: {}", id);

    OrganizationApi::delete_by_id(app_state.db_conn_ref(), id).await?;
    Ok(Json(json!({"id": id})))
}
