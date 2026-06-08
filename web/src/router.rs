use crate::middleware::throttle::{PerIpThrottle, Throttle, ThrottlePolicy};
use crate::{
    controller::{health_check_controller, oauth_callback_controller},
    middleware::auth::require_auth,
    params, protect, AppState,
};
use axum::{
    middleware::{from_fn, from_fn_with_state},
    routing::{delete, get, patch, post, put},
    Router,
};
use tower_http::services::ServeDir;

use crate::controller::{
    action_controller, agreement_controller, coaching_session, coaching_session_controller,
    goal_controller, jwt_controller, magic_link_controller, note_controller, oauth_controller,
    organization, organization_controller, password_reset_controller, tiptap_metrics_controller,
    user, user_controller, user_session_controller, webhook_controller,
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
            coaching_session_controller::create_recurring,
            coaching_session_controller::update,
            coaching_session_controller::delete,
            coaching_session::meeting_recording_controller::create,
            coaching_session::meeting_recording_controller::read,
            coaching_session::meeting_recording_controller::delete,
            coaching_session::topic_controller::index,
            coaching_session::topic_controller::create,
            coaching_session::topic_controller::update,
            coaching_session::topic_controller::reorder,
            coaching_session::topic_controller::delete,
            coaching_session::transcription_controller::read,
            coaching_session::transcription_segment_controller::index,
            health_check_controller::health_check,
            magic_link_controller::validate,
            magic_link_controller::complete_setup,
            note_controller::create,
            note_controller::update,
            note_controller::index,
            note_controller::read,
            oauth_callback_controller::callback,
            oauth_controller::authorize,
            oauth_controller::index,
            oauth_controller::read,
            oauth_controller::delete,
            organization_controller::index,
            organization_controller::read,
            organization_controller::create,
            organization_controller::update,
            organization_controller::delete,
            organization::coaching_relationship_controller::create,
            organization::coaching_relationship_controller::index,
            organization::coaching_relationship_controller::read,
            organization::coaching_relationship_controller::goal_progress,
            organization::coaching_relationship::actions_controller::read,
            organization::coaching_relationship::actions_controller::index,
            organization::user_controller::index,
            organization::user_controller::create,
            organization::user_controller::resend_invite,
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
            coaching_session::goal_controller::batch_index,
            goal_controller::coaching_sessions_by_goal,
            goal_controller::progress,
            user_controller::read,
            user_controller::update,
            user_session_controller::login,
            user_session_controller::delete,
            password_reset_controller::request,
            password_reset_controller::validate,
            password_reset_controller::complete,
            user::password_controller::update_password,
            user::organization_controller::index,
            user::action_controller::index,
            user::coaching_relationships_controller::index,
            user::coaching_session_controller::index,
            user::coaching_session_controller::counts,
            user::goal_controller::index,
            jwt_controller::generate_collab_token,
            tiptap_metrics_controller::platform_totals,
            tiptap_metrics_controller::per_org_metrics,
            tiptap_metrics_controller::abandoned_documents,
        ),
        components(
            schemas(
                crate::controller::action_controller::ActionRequest,
                crate::controller::coaching_session::meeting_recording_controller::StartRecordingParams,
                crate::controller::coaching_session::topic_controller::CreateParams,
                crate::controller::coaching_session::topic_controller::UpdateParams,
                crate::controller::coaching_session::topic_controller::ReorderParams,
                crate::controller::oauth_controller::ConnectionResponse,
                crate::controller::password_reset_controller::ValidateParams,
                crate::controller::password_reset_controller::ValidateResponse,
                crate::controller::user::coaching_session_controller::CountsResponse,
                crate::params::action::SortField,
                crate::params::agreement::SortField,
                crate::params::coaching_relationship::goal_progress::SortField,
                crate::params::coaching_session::SortField,
                crate::params::coaching_session::goal::LinkParams,
                crate::params::coaching_session::recurring::CreateRecurringParams,
                crate::params::goal::SortField,
                crate::params::sort::SortOrder,
                crate::params::user::CompleteSetupParams,
                crate::params::user::PasswordResetCompleteParams,
                crate::params::user::PasswordResetRequestParams,
                crate::params::user::goal::SortField,
                domain::action::ActionWithAssignees,
                domain::actions::Model,
                domain::agreements::Model,
                domain::coaching_relationship::CoachingRelationshipWithUserNames,
                domain::coaching_relationships::Model,
                domain::coaching_session::CountByMonth,
                domain::coaching_session::EnrichedSession,
                domain::coaching_session_topics::Model,
                domain::coaching_sessions::Model,
                domain::coaching_sessions_goals::Model,
                domain::goals::Model,
                domain::jwts::Jwt,
                domain::notes::Model,
                domain::organizations::Model,
                domain::provider::Provider,
                domain::status::Status,
                domain::user::Credentials,
                domain::users::Model,
                params::coaching_session::UpdateParams,
                params::user::UpdateParams,
                params::user::coaching_session::GroupByParam,
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
        .merge(coaching_session_meeting_recording_routes(app_state.clone()))
        .merge(coaching_session_topic_routes(app_state.clone()))
        .merge(coaching_session_transcription_routes(app_state.clone()))
        .merge(coaching_session_transcription_segment_routes(
            app_state.clone(),
        ))
        .merge(webhook_routes(app_state.clone()))
        .merge(user_routes(app_state.clone()))
        .merge(oauth_routes(app_state.clone()))
        .merge(user_password_routes(app_state.clone()))
        .merge(user_organizations_routes(app_state.clone()))
        .merge(user_actions_routes(app_state.clone()))
        .merge(user_coaching_sessions_routes(app_state.clone()))
        .merge(user_goals_routes(app_state.clone()))
        .merge(user_coaching_relationships_routes(app_state.clone()))
        .merge(magic_link_routes(app_state.clone()))
        .merge(password_reset_routes(app_state.clone()))
        .merge(user_session_routes())
        .merge(user_session_protected_routes(app_state.clone()))
        .merge(coaching_sessions_routes(app_state.clone()))
        .merge(jwt_routes(app_state.clone()))
        .merge(tiptap_metrics_routes(app_state.clone()))
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
        .route(
            "/coaching_sessions/recurring",
            post(coaching_session_controller::create_recurring),
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

/// /admin/tiptap/metrics/* - SuperAdmin-only TipTap observability
///
/// Layer order (outer -> inner): require_auth (cookie session) ->
/// admin_only (SuperAdmin role) -> handler. Axum applies layers in reverse
/// of registration; outermost-registered runs first.
pub fn tiptap_metrics_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/admin/tiptap/metrics/totals",
            get(tiptap_metrics_controller::platform_totals),
        )
        .route(
            "/admin/tiptap/metrics/per-org",
            get(tiptap_metrics_controller::per_org_metrics),
        )
        .route(
            "/admin/tiptap/metrics/abandoned",
            get(tiptap_metrics_controller::abandoned_documents),
        )
        // Per-route authorization: SuperAdmin only.
        .route_layer(from_fn_with_state(
            app_state.clone(),
            protect::tiptap_metrics::admin_only,
        ))
        // Outermost: session cookie required. Runs FIRST despite being
        // registered last (axum layer ordering).
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
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id/goal_progress",
            get(organization::coaching_relationship_controller::goal_progress),
        )
        // GET /organizations/:organization_id/coaching_relationships/actions
        // Batch endpoint — returns actions across all coaching relationships
        // where the authenticated user is the coach, with optional assignee filter
        .route(
            "/organizations/:organization_id/coaching_relationships/actions",
            get(organization::coaching_relationship::actions_controller::index),
        )
        // GET /organizations/:organization_id/coaching_relationships/:relationship_id/actions
        // Single relationship — CoachingRelationshipAccess extractor handles participant auth
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id/actions",
            get(organization::coaching_relationship::actions_controller::read),
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
            // POST /organizations/:organization_id/users/:user_id/resend-invite
            Router::new()
                .route(
                    "/organizations/:organization_id/users/:user_id/resend-invite",
                    post(organization::user_controller::resend_invite),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::organizations::users::resend_invite,
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
        .merge(
            // GET batch session goals — protected by relationship or session ownership
            Router::new()
                .route(
                    "/coaching_sessions/goals",
                    get(coaching_session::goal_controller::batch_index),
                )
                .route_layer(from_fn_with_state(
                    app_state.clone(),
                    protect::goals::batch_by_session,
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

fn magic_link_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/magic-link/validate", get(magic_link_controller::validate))
        .route(
            "/magic-link/complete-setup",
            post(magic_link_controller::complete_setup),
        )
        .with_state(app_state)
}

fn password_reset_routes(app_state: AppState) -> Router {
    // Per-IP rate limit applied to ALL password-reset endpoints. Defends
    // the endpoint surface against mass scanning (an attacker varying
    // emails or tokens per request) — orthogonal to the per-email DB rate
    // limit which defends Alice's inbox from targeted flooding. Both
    // layers are load-bearing.
    //
    // See `web::middleware::throttle` for the policy definition and
    // `docs/architecture/throttling.md` for the design and trust model.
    Router::new()
        .route(
            "/password-reset/request",
            post(password_reset_controller::request),
        )
        .route(
            "/password-reset/validate",
            post(password_reset_controller::validate),
        )
        .route(
            "/password-reset/complete",
            post(password_reset_controller::complete),
        )
        .layer(PerIpThrottle::new(ThrottlePolicy::AUTH_ENDPOINT).into_layer())
        .with_state(app_state)
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
                .route(
                    "/users/:user_id/coaching_sessions/counts",
                    get(user::coaching_session_controller::counts),
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
        .route(
            "/oauth/:provider/authorize",
            get(oauth_controller::authorize),
        )
        .route("/oauth/connections", get(oauth_controller::index))
        .route(
            "/oauth/connections/:provider",
            get(oauth_controller::read).delete(oauth_controller::delete),
        )
        .route_layer(from_fn(require_auth))
        .merge(
            // Callback doesn't require auth (user is redirected back from Google, or Zoom)
            Router::new().route(
                "/oauth/:provider/callback",
                get(oauth_callback_controller::callback),
            ),
        )
        .with_state(app_state)
}

fn coaching_session_meeting_recording_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/meeting_recording",
            get(coaching_session::meeting_recording_controller::read)
                .post(coaching_session::meeting_recording_controller::create)
                .delete(coaching_session::meeting_recording_controller::delete),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn coaching_session_topic_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/topics",
            get(coaching_session::topic_controller::index)
                .post(coaching_session::topic_controller::create),
        )
        .route(
            "/coaching_sessions/:coaching_session_id/topics/reorder",
            patch(coaching_session::topic_controller::reorder),
        )
        .route(
            "/coaching_sessions/:coaching_session_id/topics/:topic_id",
            put(coaching_session::topic_controller::update)
                .delete(coaching_session::topic_controller::delete),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn coaching_session_transcription_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/transcriptions",
            get(coaching_session::transcription_controller::read),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn coaching_session_transcription_segment_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions/:coaching_session_id/transcriptions/:transcription_id/transcription_segments",
            get(coaching_session::transcription_segment_controller::index),
        )
        .route_layer(from_fn(require_auth))
        .with_state(app_state)
}

fn webhook_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/webhooks/recall_ai", post(webhook_controller::recall_ai))
        .with_state(app_state)
}

// This will serve static files that we can use as a "fallback" for when the server panics
pub fn static_routes() -> Router {
    Router::new().nest_service("/", ServeDir::new("./"))
}
