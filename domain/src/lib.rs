//! This module re-exports various items from the `entity_api` crate.
//!
//! The purpose of this re-export is to ensure that consumers of the `domain` crate do not need to
//! directly depend on the `entity_api` crate. By re-exporting these items, we provide a clear and
//! consistent interface for working with query filters within the domain layer, while encapsulating
//! the underlying implementation details remain in the `entity_api` crate.
pub use entity_api::{
    mutate::{IntoUpdateMap, UpdateMap},
    query::{FilterOnly, IntoQueryFilterMap, QueryFilterMap},
};

// Re-exports from `entity` crate
pub use entity_api::{
    actions, agreements, coachees, coaches, coaching_relationships, coaching_sessions, jwts, notes,
    organizations, overarching_goals, query::QuerySort, users, Id, user_roles
};

pub mod action;
pub mod agreement;
pub mod coaching_relationship;
pub mod coaching_session;
pub mod emails;
pub mod error;
pub mod jwt;
pub mod note;
pub mod organization;
pub mod overarching_goal;
pub mod user;

pub(crate) mod gateway;
