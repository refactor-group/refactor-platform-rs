use serde::Serialize;
use utoipa::ToSchema;

/// Represents a JSON Web Token (JWT).
/// Note: This struct does not have a corresponding entity in the database.
///
/// This struct contains two fields:
///
/// - `token`: A string representing the JWT.
/// - `sub`: A string representing the subject of the JWT for conveniently accessing
///    the subject without having to decode the JWT.
#[derive(Serialize, Debug, ToSchema)]
#[schema(as = jwt::Jwt)] // OpenAPI schema
pub struct Jwt {
    pub token: String,
    pub sub: String,
}
