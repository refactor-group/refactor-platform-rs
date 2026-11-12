//! `coach_is_member` / `coachee_is_member` on the wire: the single relationship read, the
//! relationship a session list includes, and agreement with the create refusal.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, routing::post, Router};
use domain::{coaching_sessions, goals, users, Id};
use mockito::{Matcher, Server};
use sea_orm::{MockDatabase, Value};
use serde_json::json;
use utoipa::OpenApi;

use crate::controller::coaching_session_controller::coaching_session_create_tests::{
    config, organization, tomorrow,
};
use crate::controller::organization::coaching_relationship_controller;
use crate::controller::user::coaching_session_controller as user_coaching_session_controller;
use crate::extractors::relationship_access_test_support::{
    self as support, json_body, Caller, World,
};
use crate::router::ApiDoc;
use crate::{protect, AppState};

/// Every key the relationship object carried before the membership fields.
const RELATIONSHIP_KEYS: [&str; 7] = [
    "id",
    "organization_id",
    "coach_id",
    "coachee_id",
    "slug",
    "created_at",
    "updated_at",
];

fn routes(db: &Arc<sea_orm::DatabaseConnection>) -> Router<AppState> {
    let state = support::app_state(db);
    Router::new()
        .route(
            "/organizations/:organization_id/coaching_relationships/:relationship_id",
            get(coaching_relationship_controller::read),
        )
        .route(
            "/coaching_sessions",
            post(super::coaching_session_controller::create),
        )
        .merge(
            Router::new()
                .route(
                    "/users/:user_id/coaching_sessions",
                    get(user_coaching_session_controller::index),
                )
                .route_layer(from_fn_with_state(
                    state,
                    protect::users::coaching_sessions::index,
                )),
        )
}

fn read_uri(w: &World) -> String {
    format!(
        "/organizations/{}/coaching_relationships/{}",
        w.organization_id, w.relationship.id
    )
}

/// The relationship's row as the names-joined read returns it, with `members` holding a role.
fn listed_row(w: &World, members: &[Id]) -> BTreeMap<String, Value> {
    let r = &w.relationship;
    let name = |n: &str| Value::from(n.to_string());
    BTreeMap::from([
        ("id".to_string(), r.id.into()),
        ("organization_id".to_string(), r.organization_id.into()),
        ("coach_id".to_string(), r.coach_id.into()),
        ("coachee_id".to_string(), r.coachee_id.into()),
        ("coach_first_name".to_string(), name("Coach")),
        ("coach_last_name".to_string(), name("One")),
        ("coachee_first_name".to_string(), name("Coachee")),
        ("coachee_last_name".to_string(), name("Two")),
        ("created_at".to_string(), r.created_at.into()),
        ("updated_at".to_string(), r.updated_at.into()),
        (
            "coach_is_member".to_string(),
            members.contains(&r.coach_id).into(),
        ),
        (
            "coachee_is_member".to_string(),
            members.contains(&r.coachee_id).into(),
        ),
    ])
}

/// The relationship's model row with its membership columns, `members` holding a role.
fn included_row(w: &World, members: &[Id]) -> BTreeMap<String, Value> {
    let r = &w.relationship;
    BTreeMap::from([
        ("id".to_string(), r.id.into()),
        ("organization_id".to_string(), r.organization_id.into()),
        ("coach_id".to_string(), r.coach_id.into()),
        ("coachee_id".to_string(), r.coachee_id.into()),
        ("slug".to_string(), r.slug.clone().into()),
        ("created_at".to_string(), r.created_at.into()),
        ("updated_at".to_string(), r.updated_at.into()),
        (
            "coach_is_member".to_string(),
            members.contains(&r.coach_id).into(),
        ),
        (
            "coachee_is_member".to_string(),
            members.contains(&r.coachee_id).into(),
        ),
    ])
}

/// Membership-check rows keeping exactly `user_ids`.
fn member_rows(user_ids: &[Id]) -> Vec<BTreeMap<String, Value>> {
    user_ids
        .iter()
        .map(|&user_id| BTreeMap::from([("user_id".to_string(), user_id.into())]))
        .collect()
}

/// The coach GETs the relationship, with `members` holding a role; returns `data`.
async fn read_as_coach(w: &World, members: &[Id]) -> serde_json::Value {
    let db = Arc::new(
        w.signed_in(Caller::Coach)
            .append_query_results([vec![w.relationship.clone()]])
            .append_query_results([vec![listed_row(w, members)]])
            .into_connection(),
    );
    let response = support::send(routes(&db), &db, &w.coach.email, "GET", &read_uri(w), None).await;
    assert_eq!(response.status(), StatusCode::OK);
    json_body(response).await["data"].clone()
}

#[tokio::test]
async fn m4_05_an_included_relationship_marks_a_removed_coachee() {
    let w = support::world();
    let db = Arc::new(
        w.signed_in(Caller::Coach)
            .append_query_results([vec![w.session.clone()]])
            .append_query_results([Vec::<coaching_sessions::Model>::new()])
            .append_query_results([vec![included_row(&w, &[w.coach.id])]])
            .append_query_results([vec![w.coach.clone()]])
            .append_query_results([vec![w.coachee.clone()]])
            .append_query_results([Vec::<users::Model>::new()])
            .append_query_results([Vec::<users::Model>::new()])
            .into_connection(),
    );
    let uri = format!(
        "/users/{}/coaching_sessions?include=relationship",
        w.coach.id
    );

    let response = support::send(routes(&db), &db, &w.coach.email, "GET", &uri, None).await;

    assert_eq!(response.status(), StatusCode::OK);
    let body = json_body(response).await;
    let relationship = &body["data"][0]["relationship"];
    assert_eq!(relationship["coach_is_member"], json!(true), "{body}");
    assert_eq!(relationship["coachee_is_member"], json!(false), "{body}");
    let old = serde_json::to_value(&w.relationship).expect("the model serializes");
    for key in RELATIONSHIP_KEYS {
        assert_eq!(relationship[key], old[key], "{key}: {body}");
    }
    assert_eq!(
        relationship.as_object().map(|o| o.len()),
        Some(RELATIONSHIP_KEYS.len() + 2),
        "{body}"
    );
}

/// The coach POSTs a session, with `members` the membership rows.
async fn create_as_coach(
    w: &World,
    members: Vec<BTreeMap<String, Value>>,
    created: Option<&coaching_sessions::Model>,
) -> (StatusCode, serde_json::Value) {
    let mut server = Server::new_async().await;
    server
        .mock("POST", Matcher::Regex("^/api/documents/".to_string()))
        .with_status(200)
        .create_async()
        .await;
    let db = w
        .signed_in(Caller::Coach)
        .append_query_results([vec![w.relationship.clone()]])
        .append_query_results([vec![w.relationship.clone()]])
        .append_query_results([vec![organization(w)]])
        .append_query_results([members]);
    let db: MockDatabase = match created {
        Some(session) => db
            .append_query_results([vec![session.clone()]])
            .append_query_results([Vec::<goals::Model>::new()])
            .append_query_results([Vec::<coaching_sessions::Model>::new()]),
        None => db,
    };
    let db = Arc::new(db.into_connection());
    let body = json!({
        "coaching_relationship_id": w.relationship.id,
        "date": tomorrow(),
        "duration_minutes": 60,
    });
    let response = support::send_with(
        config(&server),
        routes(&db),
        &db,
        &w.coach.email,
        "POST",
        "/coaching_sessions",
        Some(body),
    )
    .await;
    let status = response.status();
    (status, json_body(response).await)
}

#[tokio::test]
async fn m4_07_the_field_agrees_with_the_create_refusal() {
    let w = support::world();

    let removed = [w.coach.id];
    let data = read_as_coach(&w, &removed).await;
    assert_eq!(data["coachee_is_member"], json!(false), "{data}");
    let (status, body) = create_as_coach(&w, member_rows(&removed), None).await;
    assert_eq!(status, StatusCode::CONFLICT, "{body}");
    assert_eq!(body["details"]["reason"], "coachee_not_a_member", "{body}");

    let readded = [w.coach.id, w.coachee.id];
    let data = read_as_coach(&w, &readded).await;
    assert_eq!(data["coachee_is_member"], json!(true), "{data}");
    let session = coaching_sessions::Model {
        id: Id::new_v4(),
        date: tomorrow(),
        ..w.session.clone()
    };
    let (status, body) = create_as_coach(&w, member_rows(&readded), Some(&session)).await;
    assert_eq!(status, StatusCode::OK, "{body}");
    assert_eq!(body["data"]["id"], json!(session.id), "{body}");
}

#[test]
fn both_schemas_document_the_membership_fields() {
    let spec = serde_json::to_value(ApiDoc::openapi()).expect("the spec serializes");
    let schemas = &spec["components"]["schemas"];
    for (schema, properties) in [
        (
            "domain.coaching_relationship.CoachingRelationshipWithUserNames",
            &schemas["domain.coaching_relationship.CoachingRelationshipWithUserNames"]
                ["properties"],
        ),
        (
            "domain.coaching_relationship.Membership",
            &schemas["domain.coaching_relationship.Membership"]["properties"],
        ),
    ] {
        for field in ["coach_is_member", "coachee_is_member"] {
            assert!(
                properties[field]["description"].is_string(),
                "{schema}.{field}: {properties}"
            );
        }
    }
    assert!(
        schemas["domain.coaching_relationship.CoachingRelationshipWithUserNames"]["properties"]
            ["organization_id"]
            .is_null(),
        "organization_id stays off the wire"
    );
}
