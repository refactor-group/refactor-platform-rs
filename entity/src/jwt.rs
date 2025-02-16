use serde::Serialize;

/// Represents a JSON Web Token (JWT).
///
/// This struct contains two fields:
///
/// - `token`: A string representing the JWT.
/// - `sub`: A string representing the subject of the JWT for conveniently accessing
/// the subject without having to decode the JWT.
#[derive(Serialize, Debug)]
pub struct Jwt {
    pub token: String,
    pub sub: String,
}
