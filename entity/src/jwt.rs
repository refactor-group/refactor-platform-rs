use serde::Serialize;

#[derive(Serialize, Debug)]
pub struct Jwt {
    pub token: String,
    pub sub: String,
}
