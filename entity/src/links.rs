//! Multi-hop relation definitions for SeaORM's `Linked` trait.
//!
//! Each struct here defines a chain of relations that SeaORM can
//! traverse in a single query via `find_also_linked()`.

use sea_orm::entity::prelude::*;

use crate::{
    coachees, coaches, coaching_relationships, coaching_sessions, coaching_sessions_goals,
    organizations,
};

/// coaching_sessions_goals → coaching_sessions → coaching_relationships
///
/// Allows fetching the coaching relationship for a join-table record
/// in a single query (two JOINs).
pub struct SessionGoalToCoachingRelationship;

impl Linked for SessionGoalToCoachingRelationship {
    type FromEntity = coaching_sessions_goals::Entity;
    type ToEntity = coaching_relationships::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            coaching_sessions_goals::Relation::CoachingSessions.def(),
            coaching_sessions::Relation::CoachingRelationships.def(),
        ]
    }
}

/// users (as coaches) → coaching_relationships → organizations
pub struct CoachToOrganization;

impl Linked for CoachToOrganization {
    type FromEntity = coaches::Entity;
    type ToEntity = organizations::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            coaching_relationships::Relation::Coaches.def().rev(),
            coaching_relationships::Relation::Organizations.def(),
        ]
    }
}

/// users (as coachees) → coaching_relationships → organizations
pub struct CoacheeToOrganization;

impl Linked for CoacheeToOrganization {
    type FromEntity = coachees::Entity;
    type ToEntity = organizations::Entity;

    fn link(&self) -> Vec<RelationDef> {
        vec![
            coaching_relationships::Relation::Coachees.def().rev(),
            coaching_relationships::Relation::Organizations.def(),
        ]
    }
}
