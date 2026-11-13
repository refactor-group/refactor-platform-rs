//! `coach_is_member` / `coachee_is_member` on the wire: the relationship a session list
//! includes, and the schemas that document them.

use std::collections::BTreeMap;
use std::sync::Arc;

use axum::http::StatusCode;
use axum::{middleware::from_fn_with_state, routing::get, Router};
use domain::{coaching_sessions, users, Id};
use sea_orm::Value;
use serde_json::json;
use utoipa::OpenApi;

use crate::controller::user::coaching_session_controller as user_coaching_session_controller;
use crate::extractors::relationship_access_test_support::{
    self as support, json_body, Caller, Fixture,
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
            "/users/:user_id/coaching_sessions",
            get(user_coaching_session_controller::index),
        )
        .route_layer(from_fn_with_state(
            state,
            protect::users::coaching_sessions::index,
        ))
}

/// The relationship's model row with its membership columns, `members` holding a role.
fn included_row(w: &Fixture, members: &[Id]) -> BTreeMap<String, Value> {
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

#[tokio::test]
async fn m4_05_an_included_relationship_marks_a_removed_coachee() {
    let w = support::fixture();
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
