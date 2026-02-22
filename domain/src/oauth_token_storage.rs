//! Database-backed OAuth token storage with encryption at rest.
//!
//! Implements `meeting_auth::oauth::token::Storage` using the `oauth_connections` table.
//! Tokens are encrypted with AES-256-GCM before writing and decrypted on read.

use async_trait::async_trait;
use chrono::DateTime;
use sea_orm::DatabaseConnection;
use secrecy::SecretString;

use entity_api::oauth_connection;
use meeting_auth::{
    error::{Error, ErrorKind, StorageErrorKind},
    oauth::token::{encryption, Storage, Tokens},
};

use crate::{oauth_connections::Model, provider::Provider, Id};

/// Database-backed token storage that encrypts tokens at rest.
pub struct DbOAuthTokenStorage<'db> {
    db: &'db DatabaseConnection,
    encryption_key: String,
}

impl<'db> DbOAuthTokenStorage<'db> {
    pub fn new(db: &'db DatabaseConnection, encryption_key: String) -> Self {
        Self { db, encryption_key }
    }
}

fn parse_provider(provider_id: &str) -> Result<Provider, Error> {
    match provider_id {
        "google" => Ok(Provider::Google),
        other => Err(Error {
            source: Some(other.to_string().into()),
            error_kind: ErrorKind::Storage(StorageErrorKind::Database),
        }),
    }
}

fn storage_db_err(msg: impl Into<Box<dyn std::error::Error + Send + Sync>>) -> Error {
    Error {
        source: Some(msg.into()),
        error_kind: ErrorKind::Storage(StorageErrorKind::Database),
    }
}

#[async_trait]
impl<'db> Storage for DbOAuthTokenStorage<'db> {
    async fn store(&self, user_id: &str, provider_id: &str, tokens: Tokens) -> Result<(), Error> {
        let user_id = Id::parse_str(user_id).map_err(|e| storage_db_err(e.to_string()))?;
        let provider = parse_provider(provider_id)?;
        let key = &self.encryption_key;

        let token_type = tokens.token_type.clone();
        let scopes = tokens.scopes.join(" ");
        let plain = tokens.into_plain();

        let encrypted_access = encryption::encrypt(&plain.access_token, key)
            .map_err(|e| storage_db_err(e.to_string()))?;
        let encrypted_refresh = plain
            .refresh_token
            .as_deref()
            .map(|rt| encryption::encrypt(rt, key))
            .transpose()
            .map_err(|e: Error| e)?;

        let existing = oauth_connection::find_by_user_and_provider(self.db, user_id, provider)
            .await
            .map_err(|e| storage_db_err(e.to_string()))?;

        match existing {
            Some(conn) => {
                oauth_connection::update_tokens(
                    self.db,
                    conn.id,
                    encrypted_access,
                    encrypted_refresh,
                    plain.expires_at,
                )
                .await
                .map_err(|e| storage_db_err(e.to_string()))?;
            }
            None => {
                let now = chrono::Utc::now();
                let model = Model {
                    id: Id::new_v4(),
                    user_id,
                    provider,
                    external_account_id: None,
                    external_email: None,
                    access_token: encrypted_access,
                    refresh_token: encrypted_refresh,
                    token_expires_at: plain.expires_at.map(|dt| dt.into()),
                    token_type,
                    scopes,
                    created_at: now.into(),
                    updated_at: now.into(),
                };
                oauth_connection::create(self.db, model)
                    .await
                    .map_err(|e| storage_db_err(e.to_string()))?;
            }
        }

        Ok(())
    }

    async fn get(&self, user_id: &str, provider_id: &str) -> Result<Option<Tokens>, Error> {
        let user_id = Id::parse_str(user_id).map_err(|e| storage_db_err(e.to_string()))?;
        let provider = parse_provider(provider_id)?;
        let key = &self.encryption_key;

        let conn = oauth_connection::find_by_user_and_provider(self.db, user_id, provider)
            .await
            .map_err(|e| storage_db_err(e.to_string()))?;

        let Some(conn) = conn else {
            return Ok(None);
        };

        let access_token = encryption::decrypt(&conn.access_token, key)
            .map_err(|e| storage_db_err(e.to_string()))?;
        let refresh_token = conn
            .refresh_token
            .as_deref()
            .map(|rt| encryption::decrypt(rt, key))
            .transpose()
            .map_err(|e: Error| e)?;

        let expires_at = conn.token_expires_at.map(DateTime::from);

        Ok(Some(Tokens {
            access_token: SecretString::from(access_token),
            refresh_token: refresh_token.map(SecretString::from),
            expires_at,
            token_type: conn.token_type,
            scopes: conn.scopes.split_whitespace().map(String::from).collect(),
        }))
    }

    async fn delete(&self, user_id: &str, provider_id: &str) -> Result<(), Error> {
        let user_id = Id::parse_str(user_id).map_err(|e| storage_db_err(e.to_string()))?;
        let provider = parse_provider(provider_id)?;

        oauth_connection::delete_by_user_and_provider(self.db, user_id, provider)
            .await
            .map_err(|e| storage_db_err(e.to_string()))?;

        Ok(())
    }
}
