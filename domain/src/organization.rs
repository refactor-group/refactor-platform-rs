pub use entity_api::organization::{
    archive, create, delete_by_id, find_all, find_by, find_by_id, find_by_user, unarchive, update,
    StatusFilter,
};

use crate::error::{DomainErrorKind, EntityErrorKind, Error, InternalErrorKind};
use crate::organizations;

/// Refuses an archived organization with `OrganizationArchived` (409 at the edge).
pub(crate) fn ensure_not_archived(organization: &organizations::Model) -> Result<(), Error> {
    organization.archived_at.map_or(Ok(()), |_| {
        Err(Error {
            source: None,
            error_kind: DomainErrorKind::Internal(InternalErrorKind::Entity(
                EntityErrorKind::OrganizationArchived,
            )),
        })
    })
}
