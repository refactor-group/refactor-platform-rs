//! This module provides functionality for handling JSON Web Tokens (JWTs) within the domain layer.
//! It includes the definition of claims used in JWTs, as well as functions for generating and validating tokens.
//!
//! The primary use case for this module is to generate collaboration tokens for coaching sessions,
//! which are used to authorize access to collaborative documents. The tokens include claims that specify
//! the allowed document names and other relevant information.
//!
//! The module also re-exports the `Jwt` struct from the `entity` module for convenience.
//!
//! # Example
//!
//! ```rust
//! use domain::jwt::generate_collab_token;
//! use sea_orm::DatabaseConnection;
//! use service::config::Config;
//! use entity::Id;
//!
//! async fn example(db: &DatabaseConnection, config: &Config, coaching_session_id: Id) {
//!     match generate_collab_token(db, config, coaching_session_id).await {
//!         Ok(jwt) => println!("Generated JWT: {:?}", jwt),
//!         Err(e) => eprintln!("Error generating JWT: {:?}", e),
//!     }
//! }
//! ```

use crate::coaching_session;
use crate::error::{DomainErrorKind, Error, InternalErrorKind};
use claims::TiptapCollabClaims;
use entity::Id;
use jsonwebtoken::{encode, EncodingKey, Header};
use log::*;
use sea_orm::DatabaseConnection;
use service::config::Config;

// re-export the Jwt struct from the entity module
pub use entity::jwt::Jwt;

pub(crate) mod claims;

/// Generates a collaboration token for a coaching session.
///
/// This function generates a JWT token that authorizes access to a specific collaborative document
/// associated with a coaching session. The token includes claims that specify the allowed document
/// names and other relevant information.
///

pub async fn generate_collab_token(
    db: &DatabaseConnection,
    config: &Config,
    coaching_session_id: Id,
) -> Result<Jwt, Error> {
    let coaching_session = coaching_session::find_by_id(db, coaching_session_id).await?;

    let collab_document_name = coaching_session.collab_document_name.ok_or_else(|| {
        warn!("Failed to get collab document name from coaching session");
        Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;

    // Remove the timestamp and add wildcard
    // a document name like "refactor-coaching.jim-caleb.1747304040-v0"
    // becomes "refactor-coaching.jim-caleb/*""
    let allowed_document_name_str = {
        let parts: Vec<&str> = collab_document_name.rsplitn(2, '.').collect();
        format!("{}/*", parts[1])
    };
    let tiptap_signing_key = config.tiptap_signing_key().ok_or_else(|| {
        warn!("Failed to get tiptap signing key from config");
        Error {
            source: None,
            // TODO make this InternalErrorKind::ConfigError
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Other),
        }
    })?;

    let claims = TiptapCollabClaims {
        exp: 0,
        // We'll need to add something here eventually. Potentially a company email address
        iss: "".to_string(),
        sub: collab_document_name.clone(),
        allowed_document_names: vec![allowed_document_name_str],
    };

    // Encode the claims into a JWT
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(tiptap_signing_key.as_bytes()),
    )?;

    Ok(Jwt {
        token,
        sub: collab_document_name,
    })
}
