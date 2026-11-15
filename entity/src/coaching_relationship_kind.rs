use sea_orm::entity::prelude::*;
use serde::{Deserialize, Serialize};
use utoipa::ToSchema;

/// What a coaching relationship is, which decides what each participant may do.
#[derive(
    Debug, Clone, Copy, Eq, PartialEq, EnumIter, Deserialize, Serialize, DeriveActiveEnum, ToSchema,
)]
#[serde(rename_all = "lowercase")]
#[sea_orm(
    rs_type = "String",
    db_type = "Enum",
    enum_name = "coaching_relationship_kind"
)]
#[schema(as = domain::coaching_relationship_kind::Kind)]
pub enum Kind {
    /// One person coaches the other.
    #[sea_orm(string_value = "coaching")]
    Coaching,
    /// Two equals with identical privileges.
    #[sea_orm(string_value = "colleague")]
    Colleague,
}

/// The kind given to a relationship read from a request, which never chooses its kind.
pub fn from_request() -> Kind {
    Kind::Coaching
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn kinds_serialize_lowercase() {
        assert_eq!(
            serde_json::to_value(Kind::Coaching).expect("serializes"),
            "coaching"
        );
        assert_eq!(
            serde_json::to_value(Kind::Colleague).expect("serializes"),
            "colleague"
        );
    }
}
