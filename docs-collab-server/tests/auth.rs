//! Frozen JWT authentication tests.
//!
//! Mints tokens with the same shape the application backend uses (see
//! `domain/src/jwt/claims.rs`: exp/iat/ndf/iss/sub/aud/allowedDocumentNames)
//! and asserts the server's authenticator enforces the wildcard claim, the
//! signing key, and expiration, while tolerating an `aud` claim the server
//! does not configure.

use chrono::{Duration, Utc};
use docs_collab_server::{AuthError, Authenticator, JwtAuthenticator, Scope};
use jsonwebtoken::{encode, EncodingKey, Header};
use serde::Serialize;

const SECRET: &str = "test-shared-secret-do-not-use-in-prod";
const APP_ID: &str = "tiptap_app_id_value";

#[derive(Serialize)]
struct Claims {
    exp: usize,
    iat: usize,
    ndf: usize,
    iss: String,
    sub: String,
    aud: String,
    #[serde(rename = "allowedDocumentNames")]
    allowed_document_names: Vec<String>,
}

fn mint(secret: &str, allowed_prefix: &str, sub: &str, exp_offset_secs: i64) -> String {
    let now = Utc::now().timestamp() as usize;
    let claims = Claims {
        exp: (Utc::now() + Duration::seconds(exp_offset_secs)).timestamp() as usize,
        iat: now,
        ndf: now,
        iss: "https://refactorcoach.com".into(),
        sub: sub.into(),
        aud: APP_ID.into(),
        allowed_document_names: vec![allowed_prefix.to_string()],
    };
    encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(secret.as_bytes()),
    )
    .expect("mint test token")
}

#[tokio::test]
async fn accepts_token_for_matching_wildcard() {
    let token = mint(SECRET, "org.rel.*", "org.rel.aaaa-v0", 3600);
    let auth = JwtAuthenticator::new(SECRET);
    let scope = auth
        .authenticate(&token, "org.rel.aaaa-v0")
        .await
        .expect("must authenticate");
    assert_eq!(
        scope,
        Scope {
            allowed_prefix: "org.rel.*".into()
        }
    );
}

#[tokio::test]
async fn rejects_different_org_rel() {
    let token = mint(SECRET, "org-a.rel.*", "org-a.rel.aaaa-v0", 3600);
    let auth = JwtAuthenticator::new(SECRET);
    let err = auth
        .authenticate(&token, "org-b.rel.aaaa-v0")
        .await
        .expect_err("must reject mismatched org");
    assert!(matches!(err, AuthError::ForbiddenDoc { .. }), "{err:?}");
}

#[tokio::test]
async fn rejects_same_org_different_rel() {
    let token = mint(SECRET, "org.rel-a.*", "org.rel-a.aaaa-v0", 3600);
    let auth = JwtAuthenticator::new(SECRET);
    let err = auth
        .authenticate(&token, "org.rel-b.aaaa-v0")
        .await
        .expect_err("must reject mismatched relationship");
    assert!(matches!(err, AuthError::ForbiddenDoc { .. }), "{err:?}");
}

#[tokio::test]
async fn expired_token_rejected() {
    let token = mint(SECRET, "org.rel.*", "org.rel.aaaa-v0", -3600);
    let auth = JwtAuthenticator::new(SECRET);
    let err = auth
        .authenticate(&token, "org.rel.aaaa-v0")
        .await
        .expect_err("must reject expired token");
    assert!(
        matches!(err, AuthError::Expired | AuthError::InvalidToken(_)),
        "{err:?}"
    );
}

#[tokio::test]
async fn aud_claim_does_not_cause_rejection() {
    // The app mints tokens carrying `aud` (the Tiptap app id). jsonwebtoken's
    // default validation rejects them unless `validate_aud = false`. This test
    // guards against accidentally re-enabling audience validation.
    let token = mint(SECRET, "org.rel.*", "org.rel.aaaa-v0", 3600);
    let auth = JwtAuthenticator::new(SECRET);
    auth.authenticate(&token, "org.rel.aaaa-v0")
        .await
        .expect("aud must not cause rejection");
}

#[tokio::test]
async fn wrong_signing_key_rejected() {
    let token = mint("a-different-secret", "org.rel.*", "org.rel.aaaa-v0", 3600);
    let auth = JwtAuthenticator::new(SECRET);
    let err = auth
        .authenticate(&token, "org.rel.aaaa-v0")
        .await
        .expect_err("must reject bad signature");
    assert!(matches!(err, AuthError::InvalidToken(_)), "{err:?}");
}

#[tokio::test]
async fn garbage_token_rejected() {
    let auth = JwtAuthenticator::new(SECRET);
    let err = auth
        .authenticate("not-a-jwt", "any.doc.name")
        .await
        .expect_err("must reject non-JWT input");
    assert!(matches!(err, AuthError::InvalidToken(_)), "{err:?}");
}
