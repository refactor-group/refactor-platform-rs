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
    action_controller, agreement_controller, coaching_session, coaching_session_controller,
    goal_controller, jwt_controller, note_controller, oauth_controller, organization,
    organization_controller, user, user_controller, user_session_controller,
};
use crate::sse;

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
            goal_controller::create,
            goal_controller::update,
            goal_controller::index,
            goal_controller::read,
            goal_controller::update_status,
            goal_controller::delete,
            coaching_session::goal_controller::create,
            coaching_session::goal_controller::delete,
            coaching_session::goal_controller::index,
            goal_controller::coaching_sessions_by_goal,
            goal_controller::progress,
            user_controller::update,
            user_session_controller::login,
            user_session_controller::delete,
            user::password_controller::update_password,
            user::organization_controller::index,
            user::action_controller::index,
            user::coaching_session_controller::index,
            user::goal_controller::index,
            jwt_controller::generate_collab_token,
        ),
        components(
            schemas(
                domain::actions::Model,
                domain::agreements::Model,
                domain::coaching_sessions::Model,
                domain::coaching_relationships::Model,
                domain::notes::Model,
                domain::organizations::Model,
                domain::goals::Model,
                domain::coaching_sessions_goals::Model,
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
        .merge(sse_routes(app_state.clone()))
        .merge(action_routes(app_state.clone()))
        .merge(agreement_routes(app_state.clone()))
        .merge(health_routes())
        .merge(organization_routes(app_state.clone()))
        .merge(note_routes(app_state.clone()))
        .merge(organization_coaching_relationship_routes(app_state.clone()))
        .merge(organization_user_routes(app_state.clone()))
        .merge(goal_routes(app_state.clone()))
        .merge(coaching_session_goal_routes(app_state.clone()))
        .merge(user_routes(app_state.clone()))
        .merge(oauth_routes(app_state.clone()))
        .merge(user_password_routes(app_state.clone()))
        .merge(user_organizations_routes(app_state.clone()))
        .merge(user_actions_routes(app_state.clone()))
        .merge(user_coaching_sessions_routes(app_state.clone()))
        .merge(user_goals_routes(app_state.clone()))
        .merge(user_coaching_relationships_routes(app_state.clone()))
        .merge(user_session_routes())
        .merge(user_session_protected_routes(app_state.clone()))
        .merge(coaching_sessions_routes(app_state.clone()))
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
            Router::new().route(
                "/coaching_sessions/:id",
                get(coaching_session_controller::read),
            ),
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
        // POST /organizations/:organization_id/coaching_relationships
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
            Router::new().route(
                "/organizations/:organization_id/coaching_relationships",
                get(organization::coaching_relationship_controller::index),
            ),
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
            Router::new().route(
                "/organizations/:organization_id/users",
                get(organization::user_controller::index),
            ),
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
            // DELETE /organizations/:organization_id/users/:user_id
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

pub fn goal_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/goals", post(goal_controller::create))
        .merge(
            // GET /goals — protected by coaching_relationship_id query param
            Router::new()
                .route("/goals", get(goal_controller::index))
                .route_layer(from_fn_with_state(app_state.clone(), protect::goals::index)),
        )
        .merge(
            // Routes protected by goal :id path param
            Router::new()
                .route("/goals/:id", put(goal_controller::update))
                .route("/goals/:id", delete(goal_controller::delete))
                .route("/goals/:id", get(goal_controller::read))
                .route("/goals/:id/status", put(goal_controller::update_status))
                .route(
                    "/goals/:id/sessions",
                    get(goal_controller::coaching_sessions_by_goal),
                )
                .route("/goals/:id/progress", get(goal_controller::progress))
                .route_layer(from_fn_with_state(app_state.clone(), protect::goals::by_id)),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn coaching_session_goal_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/goals",
            post(coaching_session::goal_controller::create),
        )
        .route(
            "/coaching_sessions/:coaching_session_id/goals/:id",
            delete(coaching_session::goal_controller::delete),
        )
        .merge(
            // GET goals by session — protected by coaching_session_id path param
            Router::new()
                .route(
                    "/coaching_sessions/:coaching_session_id/goals",
                    get(coaching_session::goal_controller::index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::goals::by_coaching_session_id,
                )),
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

fn user_goals_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route("/users/:user_id/goals", get(user::goal_controller::index))
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::users::goals::index,
                )),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn user_coaching_relationships_routes(app_state: AppState) -> Router {
    Router::new()
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/coaching-relationships",
                    get(user::coaching_relationships_controller::index),
                )
                .route_layer(from_fn_with_state(app_state.clone(), protect::users::read)),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn sse_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/sse", get(sse::handler::sse_handler))
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

/// Routes for Google OAuth flow and connection management
fn oauth_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/oauth/google/authorize", get(oauth_controller::authorize))
        .route("/oauth/connections", get(oauth_controller::index))
        .route(
            "/oauth/connections/:provider",
            get(oauth_controller::read).delete(oauth_controller::delete),
        )
        .route_layer(from_fn(require_auth))
        .merge(
            // Callback doesn't require auth (user is redirected back from Google)
            Router::new().route("/oauth/google/callback", get(oauth_controller::callback)),
        )
        .with_state(app_state)
}

// This will serve static files that we can use as a "fallback" for when the server panics
pub fn static_routes() -> Router {
    Router::new().nest_service("/", ServeDir::new("./"))
}
