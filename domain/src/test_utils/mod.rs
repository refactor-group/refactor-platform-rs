//! Support code shared by `domain`'s tests. See `README.md` in this directory.

pub mod audit_event;
#[cfg(all(test, feature = "mock"))]
pub(crate) mod mock;
#[cfg(test)]
pub(crate) mod proof;
pub mod sqlite;
