#![deny(
    clippy::wildcard_enum_match_arm,
    clippy::match_wildcard_for_single_variants
)]

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

impl Kind {
    /// The rules of this kind: the one place a stored kind becomes behavior.
    pub fn rules(self) -> &'static dyn Rules {
        match self {
            Kind::Coaching => &Coaching,
            Kind::Colleague => &Colleague,
        }
    }
}

/// Which column a participant occupies. Not stored and not serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    CoachId,
    CoacheeId,
}

/// What a participant is to their relationship: derived from its kind and their seat,
/// never stored. Lowercase on the wire.
#[derive(Clone, Copy, Debug, PartialEq, Eq, EnumIter, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "lowercase")]
#[schema(as = domain::coaching_relationship_kind::Role)]
pub enum Role {
    Coach,
    Coachee,
    Colleague,
}

impl Role {
    /// The role's wire spelling, as it appears in JSON, emails and filenames.
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Coach => "coach",
            Role::Coachee => "coachee",
            Role::Colleague => "colleague",
        }
    }
}

/// What a kind of relationship lets each of its participants do.
pub trait Rules {
    /// What the person in `seat` is.
    fn role(&self, seat: Seat) -> Role;
    /// May schedule, reschedule, cancel, run series, set meeting links and delete any topic.
    fn has_authority(&self, seat: Seat) -> bool;
}

/// One person coaches the other.
pub struct Coaching;

impl Rules for Coaching {
    fn role(&self, seat: Seat) -> Role {
        match seat {
            Seat::CoachId => Role::Coach,
            Seat::CoacheeId => Role::Coachee,
        }
    }

    fn has_authority(&self, seat: Seat) -> bool {
        match seat {
            Seat::CoachId => true,
            Seat::CoacheeId => false,
        }
    }
}

/// Two equals with identical privileges.
pub struct Colleague;

impl Rules for Colleague {
    fn role(&self, _seat: Seat) -> Role {
        Role::Colleague
    }

    fn has_authority(&self, _seat: Seat) -> bool {
        true
    }
}

/// The kind given to a relationship read from a request, which never chooses its kind.
pub fn from_request() -> Kind {
    Kind::Coaching
}

#[cfg(test)]
mod tests {
    use sea_orm::Iterable;

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

    #[test]
    fn coaching_gives_the_coach_seat_alone_authority() {
        let rules = Kind::Coaching.rules();
        assert_eq!(rules.role(Seat::CoachId), Role::Coach);
        assert!(rules.has_authority(Seat::CoachId));
        assert_eq!(rules.role(Seat::CoacheeId), Role::Coachee);
        assert!(!rules.has_authority(Seat::CoacheeId));
    }

    #[test]
    fn colleague_gives_both_seats_the_same_role_and_authority() {
        let rules = Kind::Colleague.rules();
        for seat in [Seat::CoachId, Seat::CoacheeId] {
            assert_eq!(rules.role(seat), Role::Colleague, "{seat:?}");
            assert!(rules.has_authority(seat), "{seat:?}");
        }
    }

    #[test]
    fn roles_serialize_lowercase() {
        for (role, wire) in [
            (Role::Coach, "coach"),
            (Role::Coachee, "coachee"),
            (Role::Colleague, "colleague"),
        ] {
            assert_eq!(serde_json::to_value(role).expect("serializes"), wire);
        }
    }

    #[test]
    fn as_str_matches_the_serde_value_of_every_role() {
        for role in Role::iter() {
            assert_eq!(
                serde_json::to_value(role).expect("serializes"),
                role.as_str(),
                "{role:?}"
            );
        }
    }
}
