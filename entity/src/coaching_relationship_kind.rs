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

    /// Whether its two seats hold different roles, so that `coach` and `coachee` each name one
    /// of the pair. Colleagues are equals, so for them neither does.
    pub fn seats_differ(self) -> bool {
        let rules = self.rules();
        rules.role(Seat::CoachId) != rules.role(Seat::CoacheeId)
    }
}

/// Which column a participant occupies. Not stored and not serialized.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Seat {
    CoachId,
    CoacheeId,
}

/// Whose zone, default session length and meeting account a session uses.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ScheduleOwner {
    /// The participant in this seat, whoever schedules.
    Seat(Seat),
    /// Whoever scheduled or last rescheduled the session or series.
    Scheduler,
}

impl ScheduleOwner {
    /// The owner's seat, given the seat of whoever scheduled; the coach seat when the scheduler
    /// holds none.
    pub fn seat(self, scheduler_seat: Option<Seat>) -> Seat {
        match self {
            ScheduleOwner::Seat(seat) => seat,
            ScheduleOwner::Scheduler => scheduler_seat.unwrap_or(Seat::CoachId),
        }
    }
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
    /// What emails and calendar invites call one of its sessions, in lower case.
    fn session_label(&self) -> &'static str;
    /// May schedule, reschedule, cancel, run series, set meeting links and delete any topic.
    fn can_schedule_sessions(&self, seat: Seat) -> bool;
    /// Sees every action in the relationship, not only their own and unassigned ones.
    fn sees_all_actions(&self, seat: Seat) -> bool;
    /// Gets the upcoming-session reminder.
    fn is_reminded(&self, seat: Seat) -> bool;
    /// Must still be in the organization for the relationship's sessions to be scheduled or
    /// changed.
    fn must_be_member_to_schedule(&self, seat: Seat) -> bool;
    /// Transcripts of its sessions are scanned for suggested actions.
    fn suggests_actions(&self) -> bool;
    /// Whose zone, default session length and meeting account a session uses.
    fn schedule_owner(&self) -> ScheduleOwner;
    /// A new meeting link may be made with the other participant's account when the schedule
    /// owner has none connected.
    fn links_with_other_account(&self) -> bool;
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

    fn session_label(&self) -> &'static str {
        "coaching session"
    }

    fn can_schedule_sessions(&self, seat: Seat) -> bool {
        match seat {
            Seat::CoachId => true,
            Seat::CoacheeId => false,
        }
    }

    fn sees_all_actions(&self, seat: Seat) -> bool {
        match seat {
            Seat::CoachId => true,
            Seat::CoacheeId => false,
        }
    }

    fn is_reminded(&self, seat: Seat) -> bool {
        match seat {
            Seat::CoachId => false,
            Seat::CoacheeId => true,
        }
    }

    fn must_be_member_to_schedule(&self, seat: Seat) -> bool {
        match seat {
            Seat::CoachId => false,
            Seat::CoacheeId => true,
        }
    }

    fn suggests_actions(&self) -> bool {
        true
    }

    fn schedule_owner(&self) -> ScheduleOwner {
        ScheduleOwner::Seat(Seat::CoachId)
    }

    fn links_with_other_account(&self) -> bool {
        false
    }
}

/// Two equals with identical privileges.
pub struct Colleague;

impl Rules for Colleague {
    fn role(&self, _seat: Seat) -> Role {
        Role::Colleague
    }

    fn session_label(&self) -> &'static str {
        "session"
    }

    fn can_schedule_sessions(&self, _seat: Seat) -> bool {
        true
    }

    fn sees_all_actions(&self, _seat: Seat) -> bool {
        true
    }

    fn is_reminded(&self, _seat: Seat) -> bool {
        true
    }

    fn must_be_member_to_schedule(&self, _seat: Seat) -> bool {
        true
    }

    fn suggests_actions(&self) -> bool {
        false
    }

    fn schedule_owner(&self) -> ScheduleOwner {
        ScheduleOwner::Scheduler
    }

    fn links_with_other_account(&self) -> bool {
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
        assert!(rules.can_schedule_sessions(Seat::CoachId));
        assert!(rules.sees_all_actions(Seat::CoachId));
        assert_eq!(rules.role(Seat::CoacheeId), Role::Coachee);
        assert!(!rules.can_schedule_sessions(Seat::CoacheeId));
        assert!(!rules.sees_all_actions(Seat::CoacheeId));
    }

    #[test]
    fn coaching_reminds_and_requires_membership_of_the_coachee_alone() {
        let rules = Kind::Coaching.rules();
        assert!(!rules.is_reminded(Seat::CoachId));
        assert!(!rules.must_be_member_to_schedule(Seat::CoachId));
        assert!(rules.is_reminded(Seat::CoacheeId));
        assert!(rules.must_be_member_to_schedule(Seat::CoacheeId));
        assert!(rules.suggests_actions());
    }

    #[test]
    fn colleague_reminds_and_requires_membership_of_both_seats_and_suggests_nothing() {
        let rules = Kind::Colleague.rules();
        for seat in [Seat::CoachId, Seat::CoacheeId] {
            assert!(rules.is_reminded(seat), "{seat:?}");
            assert!(rules.must_be_member_to_schedule(seat), "{seat:?}");
        }
        assert!(!rules.suggests_actions());
    }

    #[test]
    fn colleague_gives_both_seats_the_same_role_and_authority() {
        let rules = Kind::Colleague.rules();
        for seat in [Seat::CoachId, Seat::CoacheeId] {
            assert_eq!(rules.role(seat), Role::Colleague, "{seat:?}");
            assert!(rules.can_schedule_sessions(seat), "{seat:?}");
            assert!(rules.sees_all_actions(seat), "{seat:?}");
        }
    }

    #[test]
    fn coaching_sessions_follow_the_coach_and_link_only_with_their_account() {
        let rules = Kind::Coaching.rules();
        assert_eq!(rules.schedule_owner(), ScheduleOwner::Seat(Seat::CoachId));
        assert!(!rules.links_with_other_account());
    }

    #[test]
    fn colleague_sessions_follow_the_scheduler_and_may_link_with_the_other_account() {
        let rules = Kind::Colleague.rules();
        assert_eq!(rules.schedule_owner(), ScheduleOwner::Scheduler);
        assert!(rules.links_with_other_account());
    }

    #[test]
    fn only_coaching_seats_differ() {
        assert!(Kind::Coaching.seats_differ());
        assert!(!Kind::Colleague.seats_differ());
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
