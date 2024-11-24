use crate::{protected_resource, AppState};
use axum::{
    http::StatusCode,
    routing::{delete, get, post, put},
    Router,
};
use axum_login::login_required;
use entity_api::user::Backend;
use tower_http::services::ServeDir;

use crate::controller::{
    action_controller, agreement_controller, coaching_session_controller, note_controller,
    organization, organization_controller, overarching_goal_controller, user_controller,
    user_session_controller,
};

use crate::protect::coaching_relationships;

use utoipa::{
    openapi::security::{ApiKey, ApiKeyValue, SecurityScheme},
    Modify, OpenApi,
};
use utoipa_rapidoc::RapiDoc;

use self::organization::coaching_relationship_controller;

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
            coaching_session_controller::create,
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
            overarching_goal_controller::create,
            overarching_goal_controller::update,
            overarching_goal_controller::index,
            overarching_goal_controller::read,
            overarching_goal_controller::update_status,
            user_controller::create,
            user_session_controller::login,
            user_session_controller::logout,
        ),
        components(
            schemas(
                entity::actions::Model,
                entity::agreements::Model,
                entity::coaching_sessions::Model,
                entity::coaching_relationships::Model,
                entity::notes::Model,
                entity::organizations::Model,
                entity::overarching_goals::Model,
                entity::users::Model,
                entity_api::user::Credentials,
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
        .merge(organization_routes(app_state.clone()))
        .merge(note_routes(app_state.clone()))
        .merge(organization_coaching_relationship_routes(app_state.clone()))
        .merge(overarching_goal_routes(app_state.clone()))
        .merge(user_routes(app_state.clone()))
        .merge(user_session_routes())
        .merge(user_session_protected_routes())
        .merge(coaching_sessions_routes(app_state.clone()))
        // FIXME: protect the OpenAPI web UI
        .merge(RapiDoc::with_openapi("/api-docs/openapi2.json", ApiDoc::openapi()).path("/rapidoc"))
        .fallback_service(static_routes())
}

fn action_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/actions", post(action_controller::create))
        .route("/actions/:id", put(action_controller::update))
        .route("/actions", get(action_controller::index))
        .route("/actions/:id", get(action_controller::read))
        .route("/actions/:id/status", put(action_controller::update_status))
        .route("/actions/:id", delete(action_controller::delete))
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

fn agreement_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/agreements", post(agreement_controller::create))
        .route("/agreements/:id", put(agreement_controller::update))
        .route("/agreements", get(agreement_controller::index))
        .route("/agreements/:id", get(agreement_controller::read))
        .route("/agreements/:id", delete(agreement_controller::delete))
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

pub fn coaching_sessions_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/coaching_sessions",
            post(coaching_session_controller::create),
        )
        .route(
            "/coaching_sessions",
            get(coaching_session_controller::index),
        )
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

fn note_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/notes", post(note_controller::create))
        .route("/notes/:id", put(note_controller::update))
        .route("/notes", get(note_controller::index))
        .route("/notes/:id", get(note_controller::read))
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

fn organization_coaching_relationship_routes(app_state: AppState) -> Router {
    Router::new()
        .route(
            "/organizations/:organization_id/coaching_relationships",
            post(coaching_relationship_controller::create),
        )
        .route(
            "/organizations/:organization_id/coaching_relationships",
            get(organization::coaching_relationship_controller::index),
        )
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id",
            get(organization::coaching_relationship_controller::read),
        )
        .route_layer(login_required!(Backend, login_url = "/login"))
        .route_layer(protected_resource!(
            coaching_relationships::protect,
            StatusCode::UNAUTHORIZED
        ))
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
        .route_layer(login_required!(Backend, login_url = "/login"))
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
        .route(
            "/overarching_goals",
            get(overarching_goal_controller::index),
        )
        .route(
            "/overarching_goals/:id",
            get(overarching_goal_controller::read),
        )
        .route(
            "/overarching_goals/:id/status",
            put(overarching_goal_controller::update_status),
        )
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

pub fn user_routes(app_state: AppState) -> Router {
    Router::new()
        .route("/users", post(user_controller::create))
        .route_layer(login_required!(Backend, login_url = "/login"))
        .with_state(app_state)
}

pub fn user_session_protected_routes() -> Router {
    Router::new()
        .route("/protected", get(user_session_controller::protected))
        .route("/logout", get(user_session_controller::logout))
        .route_layer(login_required!(Backend, login_url = "/login"))
}

pub fn user_session_routes() -> Router {
    Router::new().route("/login", post(user_session_controller::login))
}

// This will serve static files that we can use as a "fallback" for when the server panics
pub fn static_routes() -> Router {
    Router::new().nest_service("/", ServeDir::new("./"))
}

#[cfg(test)]
// We need to gate seaORM's mock feature behind conditional compilation because
// the feature removes the Clone trait implementation from seaORM's DatabaseConnection.
// see https://github.com/SeaQL/sea-orm/issues/830
#[cfg(feature = "mock")]
mod organization_endpoints_tests {

    use super::*;
    use anyhow::Ok;
    use axum_login::{
        tower_sessions::{Expiry, MemoryStore, SessionManagerLayer},
        AuthManagerLayerBuilder,
    };
    use chrono::Utc;
    use entity::{organizations, users, Id};
    use entity_api::user::Backend;
    use log::{debug, LevelFilter};
    use password_auth::generate_hash;
    use reqwest::{header, header::HeaderValue, Url};
    use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase, MockExecResult};
    use serde_json::json;
    use service::{
        config::{ApiVersion, Config},
        logging::Logger,
    };
    use std::{net::SocketAddr, sync::Arc, sync::Once};
    use time::Duration;
    use tokio::net::TcpListener;

    static INIT: Once = Once::new();

    // Enable and call this at the start of a particular test to turn on DEBUG
    // level logging output used to debug a new or existing test.
    // Change to Trace to see all output.
    fn enable_test_logging(config: &mut Config) {
        INIT.call_once(|| {
            config.log_level_filter = LevelFilter::Debug;
            Logger::init_logger(&config);
        });
    }

    // A test wrapper that sets up both an http server instance with the router backend
    // endpoints and a Reqwest-based http client used to call the backend server.
    //
    // Adapted from: https://blog.sedrik.se/posts/secure-axum/
    #[derive(Clone, Debug)]
    pub struct TestClientServer {
        pub client: reqwest::Client,
        addr: String,
    }

    impl TestClientServer {
        pub async fn new(router: Router, db: &Arc<DatabaseConnection>) -> anyhow::Result<Self> {
            let session_store = MemoryStore::default();

            let session_layer = SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_expiry(Expiry::OnInactivity(Duration::days(1)));

            // Auth service
            let backend = Backend::new(db);
            let auth_layer = AuthManagerLayerBuilder::new(backend, session_layer).build();

            let listener = TcpListener::bind("0.0.0.0:0".parse::<SocketAddr>()?).await?;
            let addr = listener.local_addr()?;

            tokio::spawn(async move {
                axum::serve(listener, router.layer(auth_layer).into_make_service())
                    .await
                    .unwrap();
            });

            let mut headers = header::HeaderMap::new();
            // Note: we don't actually need to manually set the server's current AppState.config.api_version
            // since CLAP sets the default value which will always be equal to DEFAULT_API_VERSION.
            headers.insert(
                ApiVersion::field_name(),
                HeaderValue::from_static(ApiVersion::default_version()),
            );
            let client = reqwest::Client::builder()
                .cookie_store(true)
                .default_headers(headers)
                .build()?;

            Ok(Self {
                client,
                addr: addr.to_string(),
            })
        }

        pub fn url<S: AsRef<str>>(&self, path: S) -> anyhow::Result<String> {
            let base = Url::parse(format!("http://{}", self.addr).as_ref())?;
            let url = base.join(path.as_ref())?;
            Ok(url.as_str().to_string())
        }

        /// Logs into a new AuthSession with a session cookie. The cookie will be
        /// cached by the client Reqwest instance because we constructed the client
        /// with it turned on (i.e. `cookie_store(true)`).
        ///
        /// This is meant to be reused by all tests that sit behind a protected route.
        pub async fn login(&mut self, _user: &users::Model) -> anyhow::Result<()> {
            let creds = [("email", "test@domain.com"), ("password", "password2")];
            let response = self
                .client
                .post(self.url("/login").unwrap())
                .form(&creds)
                .send()
                .await?;

            debug!("response: {:?}", response);

            assert_eq!(response.status(), 200);

            Ok(())
        }

        /// Creates a test user::Model entity instance that can be used by tests to
        /// log in to the /login endpoint and create a valid AuthSession.
        pub fn get_user() -> anyhow::Result<users::Model> {
            let now = Utc::now();
            Ok(users::Model {
                id: Id::new_v4(),
                email: "test@domain.com".to_string(),
                first_name: Some("test".to_string()),
                last_name: Some("login".to_string()),
                display_name: Some("test login".to_string()),
                password: generate_hash("password2").to_owned(),
                github_username: None,
                github_profile_url: None,
                created_at: now.into(),
                updated_at: now.into(),
            })
        }
    }

    // Purpose: adds an Organization instance to a mock DB and tests the API to successfully
    // retrieve it by a specific ID and as expected and valid JSON.
    #[tokio::test]
    async fn read_returns_expected_json_for_specified_organization() -> anyhow::Result<()> {
        let mut config = Config::default();
        let now = Utc::now();
        let id = Id::new_v4();
        let endpoint_path = format!("/organizations/{}", id);

        enable_test_logging(&mut config);

        let user = TestClientServer::get_user().expect("Creating a new test user failed");
        let organization = organizations::Model {
            id: id,
            name: "Organization One".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        };

        let organization_results = [vec![organization.clone()]];

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                // initial login auth check
                .append_query_results([vec![user.clone()]])
                // check auth for the next endpoint call
                .append_query_results([vec![user.clone()]])
                .append_query_results(organization_results.clone())
                .into_connection(),
        );

        let app_state = AppState::new(config, &db);

        let mut test_client_server = TestClientServer::new(define_routes(app_state), &db)
            .await
            .unwrap();

        let response = test_client_server.login(&user).await?;

        assert_eq!(response, ()); // Make sure we get a 200 OK response

        let response = test_client_server
            .client
            .get(test_client_server.url(endpoint_path).unwrap())
            .send()
            .await?;

        // We need to parse the values to serde_json::Value to compare them
        // so that the attribute order does not matter.
        let parsed_result: serde_json::Value =
            serde_json::from_str(&response.text().await?).unwrap();

        let expected_response: serde_json::Value = json!({
                "status_code": 200,
                "data": organization
        });

        assert_eq!(parsed_result, expected_response);

        Ok(())
    }

    // Purpose: adds multiple Organization instances to a mock DB and tests the API to successfully
    // retrieve all of them as expected and valid JSON without specifying any particular ID.
    #[tokio::test]
    async fn index_returns_all_organizations() -> anyhow::Result<()> {
        let mut config = Config::default();
        let now = Utc::now();
        enable_test_logging(&mut config);

        let user = TestClientServer::get_user().expect("Creating a new test user failed");
        let organization1 = organizations::Model {
            id: Id::new_v4(),
            name: "Organization One".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        };
        let organization2 = organizations::Model {
            id: Id::new_v4(),
            name: "Organization Two".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        };
        let organization3 = organizations::Model {
            id: Id::new_v4(),
            name: "Organization Three".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        };

        // Note: for entity_api::organization::find_all() to be able to return
        // the correct query_results for the assert_eq!() below, they must all
        // be grouped together in the same inner vector.
        let organizations = [vec![organization1, organization2, organization3]];

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                //  initial login auth check
                .append_query_results([vec![user.clone()]])
                // check auth for the next endpoint call
                .append_query_results([vec![user.clone()]])
                .append_query_results(organizations.clone())
                .into_connection(),
        );

        let app_state = AppState::new(config, &db);

        let mut test_client_server = TestClientServer::new(define_routes(app_state), &db)
            .await
            .unwrap();

        let response = test_client_server.login(&user).await?;
        assert_eq!(response, ());

        let response = test_client_server
            .client
            .get(test_client_server.url("/organizations").unwrap())
            .send()
            .await?;

        // We need to parse the values to serde_json::Value to compare them
        // so that the attribute order does not matter.
        let parsed_response: serde_json::Value =
            serde_json::from_str(&response.text().await?).unwrap();
        let expected_response: serde_json::Value = json!({
            "status_code": 200,
            "data": organizations[0]
        });

        assert_eq!(parsed_response, expected_response);

        Ok(())
    }

    // Purpose: adds multiple Organization instances to a mock DB and tests that calling
    // the appropriate endpoint deletes instances specified by distinct IDs.
    #[tokio::test]
    async fn delete_an_organization_specified_by_id() -> anyhow::Result<()> {
        let mut config = Config::default();
        let now = Utc::now();
        enable_test_logging(&mut config);

        let user = TestClientServer::get_user().expect("Creating a new test user failed");
        let user_results1 = [vec![user.clone()]];

        let user_id1 = Id::new_v4();

        let organization_results1 = [vec![organizations::Model {
            id: user_id1,
            name: "Organization Two".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        }]];

        let user_id2 = Id::new_v4();
        let organization_results2 = [vec![organizations::Model {
            id: user_id2,
            name: "Organization Three".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        }]];

        let exec_results1 = [MockExecResult {
            last_insert_id: 2,
            rows_affected: 1,
        }];

        let exec_results2 = [MockExecResult {
            last_insert_id: 3,
            rows_affected: 1,
        }];

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(user_results1.clone()) // For the initial login auth check
                .append_query_results(user_results1.clone()) // For the AuthSession check done with the next endpoint call
                .append_query_results(organization_results1.clone()) // For comparing the first organization query results with
                .append_exec_results(exec_results1) // For comparing the first organization query execution results with
                .append_query_results(user_results1.clone()) // For the AuthSession check done with the next endpoint call
                .append_query_results(organization_results1.clone()) // For compare the second organization query results with
                .append_exec_results(exec_results2) // For comparing the second organization query execution results with
                .into_connection(),
        );

        let app_state = AppState::new(config, &db);

        let mut test_client_server = TestClientServer::new(define_routes(app_state), &db)
            .await
            .unwrap();

        let response = test_client_server.login(&user).await?;
        assert_eq!(response, ());

        {
            let response = test_client_server
                .client
                .delete(
                    test_client_server
                        .url(format!("/organizations/{}", user_id1))
                        .unwrap(),
                )
                .send()
                .await?
                .text()
                .await?;

            let organization2 = &organization_results1[0][0];

            assert_eq!(
                response,
                json!({
                    "id": organization2.id,
                })
                .to_string()
            );
        }

        {
            let response = test_client_server
                .client
                .delete(
                    test_client_server
                        .url(format!("/organizations/{}", user_id2))
                        .unwrap(),
                )
                .send()
                .await?
                .text()
                .await?;

            let organization3 = &organization_results2[0][0];

            assert_eq!(
                response,
                json!({
                    "id": organization3.id,
                })
                .to_string()
            );
        }

        Ok(())
    }

    // Purpose: creates multiple new Organization instances to a mock DB by calling
    // the post endpoint supplying the appropriate instance as a JSON payload.
    #[tokio::test]
    async fn create_new_organizations_successfully() -> anyhow::Result<()> {
        let mut config = Config::default();
        let now = Utc::now();
        enable_test_logging(&mut config);

        let user = TestClientServer::get_user().expect("Creating a new test user failed");
        let user_results1 = [vec![user.clone()]];

        let organization_results1 = [vec![organizations::Model {
            id: Id::new_v4(),
            name: "New Organization Five".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        }]];

        let organization_results2 = [vec![organizations::Model {
            id: Id::new_v4(),
            name: "Second Organization Six".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        }]];

        let exec_results1 = [MockExecResult {
            last_insert_id: 5,
            rows_affected: 1,
        }];

        let exec_results2 = [MockExecResult {
            last_insert_id: 6,
            rows_affected: 1,
        }];

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(user_results1.clone())
                .append_query_results(user_results1.clone())
                .append_query_results(organization_results1.clone())
                .append_exec_results(exec_results1)
                .append_query_results(user_results1.clone())
                .append_query_results(organization_results2.clone())
                .append_exec_results(exec_results2)
                .into_connection(),
        );

        let app_state = AppState::new(config, &db);

        let mut test_client_server = TestClientServer::new(define_routes(app_state), &db)
            .await
            .unwrap();

        let response = test_client_server.login(&user).await?;
        assert_eq!(response, ());

        {
            let organization5 = &organization_results1[0][0];

            let response_text = test_client_server
                .client
                .post(test_client_server.url("/organizations").unwrap())
                .json(&organization5)
                .send()
                .await?
                .text()
                .await?;

            let parsed_response: serde_json::Value = serde_json::from_str(&response_text).unwrap();

            let expected_response = json!({
                "status_code": 201,
                "data": organization5
            });

            assert_eq!(parsed_response, expected_response);
        }

        {
            let organization6 = &organization_results2[0][0];

            let response_text = test_client_server
                .client
                .post(test_client_server.url("/organizations").unwrap())
                .json(&organization6)
                .send()
                .await?
                .text()
                .await?;

            // We need to parse the values to serde_json::Value to compare them
            // so that the attribute order does not matter.
            let parsed_response: serde_json::Value = serde_json::from_str(&response_text).unwrap();

            let expected_response = json!({
                "status_code": 201,
                "data": organization6
            });

            assert_eq!(parsed_response, expected_response);
        }

        Ok(())
    }

    // Purpose: adds multiple Organization instances to a mock DB and tests that calling
    // the appropriate endpoint updates an instance specified by an ID.
    #[tokio::test]
    async fn update_an_organization_specified_by_id() -> anyhow::Result<()> {
        let mut config = Config::default();
        let now = Utc::now();
        enable_test_logging(&mut config);

        let user = TestClientServer::get_user().expect("Creating a new test user failed");
        let user_results1 = [vec![user.clone()]];

        let user_id1 = Id::new_v4();
        let user_id2 = Id::new_v4();
        let organizations = [
            vec![organizations::Model {
                id: user_id1,
                name: "Organization Two".to_owned(),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
            }],
            vec![organizations::Model {
                id: user_id2,
                name: "Updated Organization Two".to_owned(),
                created_at: now.into(),
                updated_at: now.into(),
                logo: None,
            }],
        ];

        let exec_results = [MockExecResult {
            last_insert_id: 2,
            rows_affected: 1,
        }];

        let db = Arc::new(
            MockDatabase::new(DatabaseBackend::Postgres)
                .append_query_results(user_results1.clone())
                .append_query_results(user_results1.clone())
                .append_query_results(organizations.clone())
                .append_exec_results(exec_results)
                .into_connection(),
        );

        let app_state = AppState::new(config, &db);

        let mut test_client_server = TestClientServer::new(define_routes(app_state), &db)
            .await
            .unwrap();

        let response = test_client_server.login(&user).await?;
        assert_eq!(response, ());

        let updated_organization2 = organizations::Model {
            id: user_id2,
            name: "Updated Organization Two".to_owned(),
            created_at: now.into(),
            updated_at: now.into(),
            logo: None,
        };

        let response_text = test_client_server
            .client
            .put(
                test_client_server
                    .url(format!("/organizations/{}", user_id2))
                    .unwrap(),
            )
            .json(&updated_organization2)
            .send()
            .await?
            .text()
            .await?;

        // We need to parse the values to serde_json::Value to compare them
        // so that the attribute order does not matter.
        let parsed_response: serde_json::Value =
            serde_json::from_str(&response_text).unwrap_or_default();

        let expected_response = json!({
            "status_code": 200,
            "data": updated_organization2
        });

        assert_eq!(parsed_response, expected_response);

        Ok(())
    }
}
