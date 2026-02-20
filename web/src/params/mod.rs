//! This module holds typed parameters for various endpoint inputs.
//!
//! The purpose of this module is to define and manage the parameters that are used as inputs
//! for different endpoints in the web application. By using typed parameters, we can ensure
//! that the inputs are validated (by type) and correctly formatted before they are processed by the
//! application logic.
//!
//! Each parameter type is represented by a struct or enum, which can be serialized and
//! deserialized as needed. This approach helps to maintain a clear and consistent structure
//! for endpoint inputs, making the codebase easier to understand and maintain.
//
//! ```

pub(crate) mod action;
pub(crate) mod agreement;
pub(crate) mod coaching_session;
pub(crate) mod jwt;
pub(crate) mod overarching_goal;
pub(crate) mod sort;
pub(crate) mod user;

use self::sort::SortOrder;

/// A trait for applying default sorting parameters when at least one sort parameter is provided.
///
/// This trait helps eliminate duplication in controllers by providing a standard way to apply
/// default values when either `sort_by` or `sort_order` is specified but not both.
pub(crate) trait WithSortDefaults {
    type SortField;

    /// Applies default sorting parameters when at least one sort parameter is provided.
    ///
    /// If either `sort_by` or `sort_order` is `Some`, this method ensures both have values:
    /// - `sort_by` defaults to the provided `default_field`
    /// - `sort_order` defaults to `SortOrder::Asc`
    fn apply_sort_defaults(
        sort_by: &mut Option<Self::SortField>,
        sort_order: &mut Option<SortOrder>,
        default_field: Self::SortField,
    ) {
        if sort_by.is_some() || sort_order.is_some() {
            sort_by.get_or_insert(default_field);
            sort_order.get_or_insert(SortOrder::Asc);
        }
    }
}
