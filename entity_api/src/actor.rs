use entity::Id;

/// The user performing a mutation, as recorded in an audit trail.
///
/// A newtype rather than a bare `Id` because `user_role_changes` carries no
/// foreign keys, so a transposed argument would be caught by neither the
/// compiler nor the database: it would silently attribute the change to whoever
/// that id happens to name. The other ids on those calls are FK-backed and fail
/// loudly, so only this one needs the help.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Actor(Id);

impl Actor {
    /// Marks `id` as the user to attribute an audited mutation to.
    pub fn new(id: Id) -> Self {
        Self(id)
    }

    /// The attributed user's id, for writing to an audit row.
    pub fn id(&self) -> Id {
        self.0
    }
}
