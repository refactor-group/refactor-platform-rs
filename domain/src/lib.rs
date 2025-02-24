//! This module re-exports `IntoQueryFilterMap` and `QueryFilterMap` from the `entity_api` crate.
//!
//! The purpose of this re-export is to ensure that consumers of the `domain` crate do not need to
//! directly depend on the `entity_api` crate. By re-exporting these items, we provide a clear and
//! consistent interface for working with query filters within the domain layer, while encapsulating
//! the underlying implementation details remain in the `entity_api` crate.
pub use entity_api::{IntoQueryFilterMap, QueryFilterMap};

pub mod agreement;
pub mod coaching_session;
pub mod error;
pub mod jwt;

pub(crate) mod gateway;
