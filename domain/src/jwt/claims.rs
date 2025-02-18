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
//! use domain::jwt::claims::TiptapCollabClaims;
//! use serde_json;
//!
//! let claims = TiptapCollabClaims {
//!     exp: 1625247600,
//!     iss: "issuer".to_string(),
//!     sub: "subject".to_string(),
//!     allowed_document_names: vec!["document1".to_string(), "document2".to_string()],
//! };
//!
//! let serialized_claims = serde_json::to_string(&claims).unwrap();
//! println!("Serialized claims: {}", serialized_claims);
//!
//! let deserialized_claims: TiptapCollabClaims = serde_json::from_str(&serialized_claims).unwrap();
//! println!("Deserialized claims: {:?}", deserialized_claims);
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
    pub(crate) iss: String,
    pub(crate) sub: String,
    // Titap requires this claim to be JS style case.
    #[serde(rename = "allowedDocumentNames")]
    pub(crate) allowed_document_names: Vec<String>,
}
