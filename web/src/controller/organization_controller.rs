use crate::{custom_extractors::CompareApiVersion, AppState, Error};
use axum::extract::{Path, State};
use axum::response::IntoResponse;
use axum::Json;
use entity::{organizations, Id};
use entity_api::organization as OrganizationApi;
use serde_json::json;

use log::*;

/// GET all Organizations.
#[utoipa::path(
        get,
        path = "/organizations",
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
    State(app_state): State<AppState>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all Organizations");
    let organizations = OrganizationApi::find_all(app_state.db_conn_ref()).await?;

    debug!("Found Organizations: {:?}", organizations);

    Ok(Json(organizations))
}

/// GET a particular Organization specified by its primary key.
#[utoipa::path(
    get,
    path = "/organizations/{id}",
    params(
        ("id" = i32, Path, description = "Organization id to retrieve")
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

    Ok(Json(organization))
}

/// CREATE a new Organization.
#[utoipa::path(
    post,
    path = "/organizations",
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

    Ok(Json(organization))
}

/// UPDATE a particular Organization specified by its primary key.
#[utoipa::path(
    put,
    path = "/organizations/{id}",
    params(
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
    Json(organization_model): Json<entity::organizations::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "UPDATE the entire Organization by id: {:?}, new name: {:?}",
        id, organization_model.name
    );

    let updated_organization: organizations::Model =
        OrganizationApi::update(app_state.db_conn_ref(), id, organization_model).await?;

    Ok(Json(updated_organization))
}

/// DELETE an Organization specified by its primary key.
#[utoipa::path(
    delete,
    path = "/organizations/{id}",
    params(
        ("id" = i32, Path, description = "Organization id to update")
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
