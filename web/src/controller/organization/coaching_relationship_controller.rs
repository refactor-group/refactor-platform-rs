use crate::controller::ApiResponse;
use crate::extractors::{
    authenticated_user::AuthenticatedUser, compare_api_version::CompareApiVersion,
};
use crate::{AppState, Error};
use axum::extract::{Path, State};
use axum::http::StatusCode;
use axum::response::IntoResponse;
use axum::Json;
use domain::coaching_relationship::CoachingRelationshipWithUserNames;
use domain::{coaching_relationship as CoachingRelationshipApi, coaching_relationships, Id};
use service::config::ApiVersion;

use log::*;

/// CREATE a new CoachingRelationship.
#[utoipa::path(
    post,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
    ),
    request_body = entity::coaching_relationships::Model,
    responses(
        (status = 200, description = "Successfully created a new Coaching Relationship", body = [coaching_relationships::Model]),
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
    Json(coaching_relationship_model): Json<coaching_relationships::Model>,
) -> Result<impl IntoResponse, Error> {
    debug!(
        "CREATE new Coaching Relationship from: {:?}",
        coaching_relationship_model
    );

    let coaching_relationship: coaching_relationships::Model =
        CoachingRelationshipApi::create(app_state.db_conn_ref(), coaching_relationship_model)
            .await?;

    debug!(
        "Newly created Coaching Relationship: {:?}",
        &coaching_relationship
    );

    Ok(Json(ApiResponse::new(
        StatusCode::CREATED.into(),
        coaching_relationship,
    )))
}

/// GET a particular CoachingRelationship specified by the organization Id and relationship Id.
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships/{relationship_id}",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve the CoachingRelationship under"),
        ("relationship_id" = String, Path, description = "CoachingRelationship id to retrieve")
    ),
    responses(
        (status = 200, description = "Successfully retrieved a certain CoachingRelationship by its id", body = [coaching_relationships::Model]),
        (status = 401, description = "Unauthorized"),
        (status = 404, description = "CoachingRelationship not found"),
        (status = 405, description = "Method not allowed")
    ),
    security(
        ("cookie_auth" = [])
    )
)]
pub async fn read(
    CompareApiVersion(_v): CompareApiVersion,
    AuthenticatedUser(_user): AuthenticatedUser,
    // TODO: create a new Extractor to authorize the user to access
    // the data requested
    State(app_state): State<AppState>,
    Path((_organization_id, relationship_id)): Path<(Id, Id)>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET CoachingRelationship by id: {}", relationship_id);

    let relationship: Option<CoachingRelationshipWithUserNames> =
        CoachingRelationshipApi::get_relationship_with_user_names(
            app_state.db_conn_ref(),
            relationship_id,
        )
        .await?;

    Ok(Json(ApiResponse::new(StatusCode::OK.into(), relationship)))
}

/// GET all CoachingRelationships by organization_id
#[utoipa::path(
    get,
    path = "/organizations/{organization_id}/coaching_relationships",
    params(
        ApiVersion,
        ("organization_id" = Id, Path, description = "Organization id to retrieve CoachingRelationships")
    ),
    responses(
        (status = 200, description = "Successfully retrieved all CoachingRelationships", body = [coaching_relationships::Model]),
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
    Path(organization_id): Path<Id>,
) -> Result<impl IntoResponse, Error> {
    debug!("GET all CoachingRelationships");
    let coaching_relationships = CoachingRelationshipApi::find_by_organization_with_user_names(
        app_state.db_conn_ref(),
        organization_id,
        user.id,
    )
    .await?;

    debug!("Found CoachingRelationships: {:?}", coaching_relationships);

    Ok(Json(ApiResponse::new(
        StatusCode::OK.into(),
        coaching_relationships,
    )))
}
