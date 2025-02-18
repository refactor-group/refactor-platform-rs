//! This module provides protection mechanisms for various resources in the web application.
//!
//! It includes submodules for authorizing access to resources. Each submodule contains the necessary logic to protect
//! the corresponding resources, ensuring that only authorized users can access or modify them.
//!
//! The protection mechanisms are designed to be flexible and extensible, allowing for the addition
//! of new resources and protection strategies as needed. By organizing the protection logic into
//! separate submodules, we can maintain a clear and modular structure, making the codebase easier
//! to understand and maintain.

pub(crate) mod actions;
pub(crate) mod agreements;
pub(crate) mod coaching_relationships;
pub(crate) mod coaching_sessions;
pub(crate) mod jwt;
pub(crate) mod notes;
pub(crate) mod overarching_goals;
