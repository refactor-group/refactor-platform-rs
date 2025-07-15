//! This module defines the claims used in JSON Web Tokens (JWTs) within the domain layer.
//!
//! It provides structures for various types of claims that can be included in JWTs, such as
//! those used for Tiptap collaboration. Each claim type is represented by a struct that can
//! be serialized and deserialized for use in JWTs.
//!
//! The module is designed to be extensible, allowing for the addition of new claim types
//! as needed. The current implementation includes `TiptapCollabClaims`, which contains
//! fields like expiration time, issuer, subject, and allowed document names.
//!
//! # Example
//!
//! ```rust
//! // This module provides internal claims structures for JWT tokens.
//! // TiptapCollabClaims is used internally by the jwt module for generating
//! // collaboration tokens via the generate_collab_token function.
//! use serde_json;
//!
//! // Example of working with JSON claims data
//! let claims_json = r#"{
//!     "exp": 1825247600,
//!     "iat": 1625247600,
//!     "ndf": 1625247600,
//!     "iss": "issuer",
//!     "sub": "subject",
//!     "allowedDocumentNames": ["document1", "document2"]
//! }"#;
//!
//! let parsed: serde_json::Value = serde_json::from_str(claims_json).unwrap();
//! println!("Parsed claims: {}", parsed);
//! ```

use serde::{Deserialize, Serialize};

/// Represents the claims for a Tiptap collaboration token.
///
/// This struct is used to serialize and deserialize the claims for a Tiptap collaboration token.
///
/// The `TiptapCollabClaims` struct contains the following fields:
///
#[derive(Debug, Serialize, Deserialize)]
pub(crate) struct TiptapCollabClaims {
    pub(crate) exp: usize,
    pub(crate) iat: usize,
    pub(crate) ndf: usize,
    pub(crate) iss: String,
    pub(crate) sub: String,
    pub(crate) aud: String,
    // Titap requires this claim to be JS style case.
    #[serde(rename = "allowedDocumentNames")]
    pub(crate) allowed_document_names: Vec<String>,
}
