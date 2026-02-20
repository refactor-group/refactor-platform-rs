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

// Re-exports from `entity` crate via `entity_api`
pub use entity_api::{
    actions, agreements, coachees, coaches, coaching_relationships, coaching_sessions, jwts, notes,
    oauth_connections, organizations, overarching_goals, provider, query::QuerySort, status,
    user_roles, users, Id,
};

pub mod action;
pub mod agreement;
pub mod coaching_relationship;
pub mod coaching_session;
pub mod emails;
pub mod encryption;
pub mod error;
pub mod jwt;
pub mod note;
pub mod oauth_connection;
pub mod organization;
pub mod overarching_goal;
pub mod user;

pub mod gateway;
