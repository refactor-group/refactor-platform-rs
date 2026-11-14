//! Support code shared by `domain`'s tests. See `README.md` in this directory.

#[cfg(all(test, feature = "mock"))]
pub(crate) mod mock;
pub mod sqlite;
