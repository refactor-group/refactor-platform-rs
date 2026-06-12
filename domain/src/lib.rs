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
    actions, agreements, coachees, coaches, coaching_relationships, coaching_session_topics,
    coaching_sessions, coaching_sessions_goals, duration, goals, jwts, magic_link_tokens, notes,
    oauth_connections, organizations, password_reset_attempts, provider, query::QuerySort, status,
    token_purpose, topic_priority, topic_status, user_roles, users, Id,
};

pub mod action;
pub mod agreement;
pub mod coaching_relationship;
pub mod coaching_session;
pub(crate) mod coaching_session_goal;
mod coaching_session_hydration;
pub mod coaching_session_topic;
pub mod emails;
pub mod error;
pub mod goal;
pub mod goal_progress;
pub mod jwt;
pub mod magic_link_token;
pub mod meeting_recording;
pub mod note;

pub mod oauth_connection;
pub mod oauth_token_storage;
pub mod organization;
pub mod password_policy;
pub mod password_reset;
pub mod tiptap_metrics;
pub mod transcript_segment;
pub mod transcription;
pub mod user;

pub mod gateway;
pub mod webhook;

#[cfg(all(test, feature = "mock"))]
mod test_support;

// Re-export events crate as the events module to maintain existing API
pub use events;
