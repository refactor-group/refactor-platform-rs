//! Support code shared by `domain`'s tests. See `README.md` in this directory.

#[cfg(all(test, feature = "mock"))]
pub(crate) mod mock;
#[cfg(test)]
pub(crate) mod proof;
pub mod sqlite;
