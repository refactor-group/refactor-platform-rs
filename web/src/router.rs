use crate::{
    controller::health_check_controller, middleware::auth::require_auth, params, protect, AppState,
};
use axum::{
    middleware::{from_fn, from_fn_with_state},
    routing::{delete, get, post, put},
    Router,
};
use tower_http::services::ServeDir;

use crate::controller::{
    action_controller, agreement_controller, coaching_relationship_controller,
    coaching_session_controller, integration_controller, jwt_controller,
    meeting_recording_controller, note_controller, oauth_controller, organization,
    organization_controller, overarching_goal_controller, user, user_controller,
    user_session_controller, webhook_controller,
};

use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_rapidoc::RapiDoc;

// This is the global definition of our OpenAPI spec. To be a part
// of the rendered spec, a path and schema must be listed here.
#[derive(OpenApi)]
#[openapi(
        info(
            title = "Refactor Platform API"
        ),
        paths(
            action_controller::create,
            action_controller::update,
            action_controller::index,
            action_controller::read,
            action_controller::update_status,
            action_controller::delete,
            agreement_controller::create,
            agreement_controller::update,
            agreement_controller::index,
            agreement_controller::read,
            agreement_controller::delete,
            coaching_session_controller::index,
            coaching_session_controller::read,
            coaching_session_controller::create,
            coaching_session_controller::update,
            coaching_session_controller::delete,
            note_controller::create,
            note_controller::update,
            note_controller::index,
            note_controller::read,
            organization_controller::index,
            organization_controller::read,
            organization_controller::create,
            organization_controller::update,
            organization_controller::delete,
            organization::coaching_relationship_controller::create,
            organization::coaching_relationship_controller::index,
            organization::coaching_relationship_controller::read,
            organization::user_controller::index,
            organization::user_controller::create,
            organization::user_controller::delete,
            overarching_goal_controller::create,
            overarching_goal_controller::update,
            overarching_goal_controller::index,
            overarching_goal_controller::read,
            overarching_goal_controller::update_status,
            user_controller::update,
            user_session_controller::login,
            user_session_controller::delete,
            user::password_controller::update_password,
            user::organization_controller::index,
            user::action_controller::index,
            user::coaching_session_controller::index,
            user::overarching_goal_controller::index,
            jwt_controller::generate_collab_token,
            meeting_recording_controller::get_recording_status,
            meeting_recording_controller::start_recording,
            meeting_recording_controller::stop_recording,
        ),
        components(
            schemas(
                domain::actions::Model,
                domain::agreements::Model,
                domain::coaching_sessions::Model,
                domain::coaching_relationships::Model,
                domain::meeting_recordings::Model,
                domain::notes::Model,
                domain::organizations::Model,
                domain::overarching_goals::Model,
                domain::users::Model,
                domain::user::Credentials,
                params::user::UpdateParams,
                params::coaching_session::UpdateParams,
            )
        ),
        modifiers(&SecurityAddon),
        tags(
            (name = "refactor_platform", description = "Refactor Coaching & Mentorship API")
        )
    )]
struct ApiDoc;

struct SecurityAddon;

// Defines our cookie session based authentication requirement for gaining access to our
// API endpoints for OpenAPI.
impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut utoipa::openapi::OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "cookie_auth",
                SecurityScheme::ApiKey(ApiKey::Cookie(ApiKeyValue::with_description(
                    "id",
                    "Session id value returned from successful login via Set-Cookie header",
                ))),
            )
        }
    }
}

pub fn define_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(action_routes(app_state.clone()))
        .merge(agreement_routes(app_state.clone()))
        .merge(coaching_relationship_routes(app_state.clone()))
        .merge(health_routes())
        .merge(organization_routes(app_state.clone()))
        .merge(note_routes(app_state.clone()))
        .merge(organization_coaching_relationship_routes(app_state.clone()))
        .merge(organization_user_routes(app_state.clone()))
        .merge(overarching_goal_routes(app_state.clone()))
        .merge(user_routes(app_state.clone()))
        .merge(user_integrations_routes(app_state.clone()))
        .merge(oauth_routes(app_state.clone()))
        .merge(user_password_routes(app_state.clone()))
        .merge(user_organizations_routes(app_state.clone()))
        .merge(user_actions_routes(app_state.clone()))
        .merge(user_coaching_sessions_routes(app_state.clone()))
        .merge(user_overarching_goals_routes(app_state.clone()))
        .merge(user_session_routes())
        .merge(user_session_protected_routes(app_state.clone()))
        .merge(coaching_sessions_routes(app_state.clone()))
        .merge(meeting_recording_routes(app_state.clone()))
        .merge(webhook_routes(app_state.clone()))
        .merge(jwt_routes(app_state.clone()))
        // **** FIXME: protect the OpenAPI web UI
        .merge(RapiDoc::with_openapi("/api-docs/openapi2.json", ApiDoc::openapi()).path("/rapidoc"))
        .fallback_service(static_routes())
}

fn action_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/actions", post(action_controller::create))
        .route("/actions/:id", put(action_controller::update))
        .route("/actions/:id", get(action_controller::read))
        .route("/actions/:id/status", put(action_controller::update_status))
        .route("/actions/:id", delete(action_controller::delete))
        .merge(
            // GET /actions
            Router::new()
                .route("/actions", get(action_controller::index))
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::actions::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn agreement_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/agreements", post(agreement_controller::create))
        .route("/agreements/:id", put(agreement_controller::update))
        .merge(
            // GET /agreements
            Router::new()
                .route("/agreements", get(agreement_controller::index))
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::agreements::index,
                )),
        )
        .route("/agreements/:id", get(agreement_controller::read))
        .route("/agreements/:id", delete(agreement_controller::delete))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn coaching_sessions_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions",
            post(coaching_session_controller::create),
        )
        .merge(
            // Get /coaching_sessions
            Router::new()
                .route(
                    "/coaching_sessions",
                    get(coaching_session_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::coaching_sessions::index,
                )),
        )
        .merge(
            // GET /coaching_sessions/:id
            Router::new()
                .route(
                    "/coaching_sessions/:id",
                    get(coaching_session_controller::read),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::coaching_sessions::read,
                )),
        )
        .merge(
            // PUT /coaching_sessions/:id
            Router::new()
                .route(
                    "/coaching_sessions/:id",
                    put(coaching_session_controller::update),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::coaching_sessions::update,
                )),
        )
        .merge(
            // DELETE /coaching_sessions
            Router::new()
                .route(
                    "/coaching_sessions/:id",
                    delete(coaching_session_controller::delete),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::coaching_sessions::delete,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn health_routes() -> Router {
    Router::new().route("/health", get(health_check_controller::health_check))
}

fn note_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/notes", post(note_controller::create))
        .route("/notes/:id", put(note_controller::update))
        .merge(
            // GET /notes
            Router::new()
                .route("/notes", get(note_controller::index))
                .route_layer(from_fn_with_state(app_state.clone(), protect::notes::index)),
        )
        .route("/notes/:id", get(note_controller::read))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn organization_coaching_relationship_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/organizations/:organization_id/coaching_relationships",
            post(organization::coaching_relationship_controller::create),
        )
        .route_layer(from_fn_with_state(
            app_state.clone(),
            protect::organizations::coaching_relationships::create,
        ))
        .merge(
            // GET /organizations/:organization_id/coaching_relationships
            Router::new()
                .route(
                    "/organizations/:organization_id/coaching_relationships",
                    get(organization::coaching_relationship_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::organizations::coaching_relationships::index,
                )),
        )
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id",
            get(organization::coaching_relationship_controller::read),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn organization_user_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            // GET /organizations/:organization_id/users
            Router::new()
                .route(
                    "/organizations/:organization_id/users",
                    get(organization::user_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::organizations::users::index,
                )),
        )
        .merge(
            // POST /organizations/:organization_id/users
            Router::new()
                .route(
                    "/organizations/:organization_id/users",
                    post(organization::user_controller::create),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::organizations::users::create,
                )),
        )
        .merge(
            // POST /organizations/:organization_id/users
            Router::new()
                .route(
                    "/organizations/:organization_id/users/:user_id",
                    delete(organization::user_controller::delete),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::organizations::users::delete,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}
pub fn organization_routes(app_state: AppState) -> Router {
    Router::new()
        // The goal will be able to do something like the follow Node.js code does for
        // versioning: https://www.codemzy.com/blog/nodejs-api-versioning
        // except we can use axum-extras `or` like is show here:
        // https://gist.github.com/davidpdrsn/eb4e703e7e068ece3efd975b8f6bc340#file-content_type_or-rs-L17
        .route("/organizations", get(organization_controller::index))
        .route("/organizations/:id", get(organization_controller::read))
        .route("/organizations", post(organization_controller::create))
        .route("/organizations/:id", put(organization_controller::update))
        .route(
            "/organizations/:id",
            delete(organization_controller::delete),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn overarching_goal_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/overarching_goals",
            post(overarching_goal_controller::create),
        )
        .route(
            "/overarching_goals/:id",
            put(overarching_goal_controller::update),
        )
        .merge(
            // GET /overarching_goals
            Router::new()
                .route(
                    "/overarching_goals",
                    get(overarching_goal_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::overarching_goals::index,
                )),
        )
        .route(
            "/overarching_goals/:id",
            get(overarching_goal_controller::read),
        )
        .route(
            "/overarching_goals/:id/status",
            put(overarching_goal_controller::update_status),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn user_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            // GET /users/:id
            Router::new()
                .route("/users/:id", get(user_controller::read))
                .route_layer(from_fn_with_state(app_state.clone(), protect::users::read)),
        )
        .merge(
            // PUT /users/:id
            Router::new()
                .route("/users/:id", put(user_controller::update))
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::update,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn user_password_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/users/:id/password",
            put(user::password_controller::update_password),
        )
        .route_layer(from_fn_with_state(
            app_state.clone(),
            protect::users::passwords::update_password,
        ))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn user_session_protected_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/delete", delete(user_session_controller::delete))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

pub fn user_session_routes() -> Router {
    Router::new().route("/login", post(user_session_controller::login))
}

fn jwt_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/jwt/generate_collab_token",
            get(jwt_controller::generate_collab_token),
        )
        .route_layer(from_fn_with_state(
            app_state.clone(),
            protect::jwt::generate_collab_token,
        ))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn user_organizations_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/organizations",
                    get(user::organization_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::organizations::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn user_actions_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/actions",
                    get(user::action_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::actions::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn user_coaching_sessions_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/coaching_sessions",
                    get(user::coaching_session_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::coaching_sessions::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn user_overarching_goals_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/overarching_goals",
                    get(user::overarching_goal_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::overarching_goals::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

/// Routes for user integrations (API key management)
fn user_integrations_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/users/:user_id/integrations",
            get(integration_controller::read),
        )
        .route(
            "/users/:user_id/integrations",
            put(integration_controller::update),
        )
        .route(
            "/users/:user_id/integrations/verify/recall-ai",
            post(integration_controller::verify_recall_ai),
        )
        .route(
            "/users/:user_id/integrations/verify/assembly-ai",
            post(integration_controller::verify_assembly_ai),
        )
        .route(
            "/users/:user_id/integrations/google",
            delete(integration_controller::disconnect_google),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

/// Routes for coaching relationships (direct, non-nested operations)
fn coaching_relationship_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_relationships/:id",
            put(coaching_relationship_controller::update),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

/// Routes for Google OAuth flow
fn oauth_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/oauth/google/authorize", get(oauth_controller::authorize))
        .route_layer(from_fn(require_auth))
        .merge(
            // Callback doesn't require auth (user is redirected back from Google)
            Router::new().route("/oauth/google/callback", get(oauth_controller::callback)),
        )
        .with_state(app_state)
}

/// Routes for meeting recording operations
fn meeting_recording_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:id/recording",
            get(meeting_recording_controller::get_recording_status),
        )
        .route(
            "/coaching_sessions/:id/recording/start",
            post(meeting_recording_controller::start_recording),
        )
        .route(
            "/coaching_sessions/:id/recording/stop",
            post(meeting_recording_controller::stop_recording),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

/// Routes for external service webhooks (no authentication - validated by webhook secret)
fn webhook_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/webhooks/recall", post(webhook_controller::recall_webhook))
        .with_state(app_state)
}

// This will serve static files that we can use as a "fallback" for when the server panics
pub fn static_routes() -> Router {
    Router::new().nest_service("/", ServeDir::new("./"))
}
