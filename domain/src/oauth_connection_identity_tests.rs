//! A connection's platform account id: recorded on connect, and only looked up when missing.

use meeting_auth::oauth::token::Plain;
use meeting_auth::oauth::UserInfo;
use sea_orm::{DatabaseBackend, DatabaseConnection, MockDatabase};
use service::config::Config;

use super::*;

fn connection(account_id: Option<&str>) -> OauthConnectionModel {
    let now = chrono::Utc::now();
    OauthConnectionModel {
        id: Id::new_v4(),
        user_id: Id::new_v4(),
        provider: MeetingProvider::Google,
        external_account_id: account_id.map(str::to_string),
        external_email: Some("coach@example.com".to_string()),
        access_token: "encrypted-access".to_string(),
        refresh_token: Some("encrypted-refresh".to_string()),
        token_expires_at: Some(now.into()),
        token_type: "Bearer".to_string(),
        scopes: "openid email".to_string(),
        created_at: now.into(),
        updated_at: now.into(),
    }
}

fn user_info(id: &str) -> UserInfo {
    UserInfo {
        id: id.to_string(),
        email: "coach@example.com".to_string(),
        name: None,
        picture: None,
        email_verified: Some(true),
    }
}

fn plain_tokens() -> Plain {
    Plain {
        access_token: "access".to_string(),
        refresh_token: None,
        expires_at: None,
    }
}

fn statements(db: DatabaseConnection) -> Vec<String> {
    db.into_transaction_log()
        .iter()
        .flat_map(|transaction| {
            transaction
                .statements()
                .iter()
                .map(|statement| statement.sql.clone())
                .collect::<Vec<_>>()
        })
        .collect()
}

/// No connection for the provider means no account id, and nothing else is attempted.
#[tokio::test]
async fn no_connection_means_no_account_id() -> Result<(), Error> {
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results::<OauthConnectionModel, Vec<OauthConnectionModel>, _>(vec![vec![]])
        .into_connection();

    let account_id = external_account_id(
        &db,
        &Config::default(),
        Id::new_v4(),
        MeetingProvider::Google,
    )
    .await?;

    assert_eq!(account_id, None);
    assert_eq!(statements(db).len(), 1);
    Ok(())
}

/// A stored account id is returned as is: no token fetch (which would fail here without an
/// encryption key) and no write.
#[tokio::test]
async fn a_stored_account_id_is_returned_without_a_lookup() -> Result<(), Error> {
    let stored = connection(Some("google-123"));
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![stored.clone()]])
        .into_connection();

    let account_id = external_account_id(
        &db,
        &Config::default(),
        stored.user_id,
        MeetingProvider::Google,
    )
    .await?;

    assert_eq!(account_id.as_deref(), Some("google-123"));
    let sql = statements(db);
    assert_eq!(sql.len(), 1);
    assert!(!sql.iter().any(|statement| statement.starts_with("UPDATE")));
    Ok(())
}

/// A missing account id is looked up with the user's token; without one, it errors and writes nothing.
#[tokio::test]
async fn a_missing_account_id_needs_a_valid_token() {
    let stored = connection(None);
    let db = MockDatabase::new(DatabaseBackend::Postgres)
        .append_query_results(vec![vec![stored.clone()]])
        .into_connection();

    let result = external_account_id(
        &db,
        &Config::default(),
        stored.user_id,
        MeetingProvider::Google,
    )
    .await;

    assert!(result.is_err());
    assert!(!statements(db)
        .iter()
        .any(|statement| statement.starts_with("UPDATE")));
}

/// A new Google connection records the Google account id, not only the email.
#[test]
fn a_new_google_connection_records_the_account_id() {
    let model = create_oauth_connection_model(
        Id::new_v4(),
        MeetingProvider::Google,
        user_info("google-123"),
        &plain_tokens(),
        "openid email".to_string(),
        "encrypted-access".to_string(),
        None,
    );

    assert_eq!(model.external_account_id.as_deref(), Some("google-123"));
    assert_eq!(model.external_email.as_deref(), Some("coach@example.com"));
}

/// Zoom connections keep recording their account id.
#[test]
fn a_new_zoom_connection_records_the_account_id() {
    let model = create_oauth_connection_model(
        Id::new_v4(),
        MeetingProvider::Zoom,
        user_info("zoom-456"),
        &plain_tokens(),
        "meeting:write".to_string(),
        "encrypted-access".to_string(),
        None,
    );

    assert_eq!(model.external_account_id.as_deref(), Some("zoom-456"));
    assert_eq!(model.external_email.as_deref(), Some("coach@example.com"));
}
