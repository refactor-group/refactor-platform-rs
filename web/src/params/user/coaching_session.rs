use chrono::NaiveDate;
use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use domain::{coaching_sessions, Id, QuerySort};

/// Related resources that can be batch-loaded with coaching sessions.
///
/// Used in `?include=` query parameter to eliminate N+1 queries. Supports
/// comma-separated values: `?include=relationship,organization,goal,agreements`
///
/// Maps to `entity_api::coaching_session::IncludeOptions` for database queries.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash)]
#[serde(rename_all = "lowercase")]
pub enum IncludeParam {
    /// Include coaching relationship (coach/coachee info)
    Relationship,
    /// Include organization (requires relationship)
    Organization,
    /// Include goal
    Goal,
    /// Include session agreements
    Agreements,
    /// Include session topics
    Topics,
}

/// Query parameters for GET `/users/{user_id}/coaching_sessions` endpoint.
///
/// Supports date range filtering, sorting, and optional batch loading of related resources.
/// The enhanced `include` parameter enables efficient data fetching (see `IncludeParam`).
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    /// User ID from URL path (not a query parameter)
    #[serde(skip)]
    pub(crate) user_id: Id,
    /// Optional: filter sessions to only those in this coaching relationship
    pub(crate) coaching_relationship_id: Option<Id>,
    /// Optional: filter sessions starting from this date (inclusive).
    ///
    /// Interpreted in `tz` when present; otherwise UTC.
    pub(crate) from_date: Option<NaiveDate>,
    /// Optional: filter sessions up to this date (inclusive at calendar-day precision).
    ///
    /// Interpreted in `tz` when present; otherwise UTC.
    pub(crate) to_date: Option<NaiveDate>,
    /// Optional: IANA timezone for evaluating `from_date`/`to_date` as
    /// calendar-day boundaries in that zone. Omitted = UTC-naive boundaries.
    ///
    /// Validated in the handler via `chrono_tz::Tz::from_str`; invalid → 400
    /// `invalid_timezone`. Kept as `String` so the offending value is
    /// preserved for the structured discriminator response.
    pub(crate) tz: Option<String>,
    /// Optional: field to sort by (e.g., "date", "created_at")
    pub(crate) sort_by: Option<SortField>,
    /// Optional: sort direction (asc/desc)
    pub(crate) sort_order: Option<SortOrder>,
    /// Optional: comma-separated list of related resources to batch-load
    ///
    /// Example: `?include=relationship,organization,goal`
    ///
    /// See `IncludeParam` for valid values and N+1 query optimization details.
    #[serde(default, deserialize_with = "deserialize_comma_separated")]
    pub(crate) include: Vec<IncludeParam>,
}

/// Custom deserializer for comma-separated include parameter
fn deserialize_comma_separated<'de, D>(deserializer: D) -> Result<Vec<IncludeParam>, D::Error>
where
    D: serde::Deserializer<'de>,
{
    let s: Option<String> = Option::deserialize(deserializer)?;
    match s {
        None => Ok(Vec::new()),
        Some(s) if s.is_empty() => Ok(Vec::new()),
        Some(s) => s
            .split(',')
            .map(str::trim)
            .filter(|s| !s.is_empty())
            .map(|s| {
                serde_json::from_value(serde_json::Value::String(s.to_string()))
                    .map_err(serde::de::Error::custom)
            })
            .collect(),
    }
}

impl IndexParams {
    /// Sets the user_id field (useful when user_id comes from path parameter).
    ///
    /// This allows using `Query<IndexParams>` to deserialize query parameters,
    /// then setting the path-based user_id afterward for consistency with other
    /// user sub-resource endpoints.
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
        self
    }

    /// Applies default sorting parameters if any sort parameter is provided.
    ///
    /// Uses `Date` as the default sort field for coaching sessions.
    /// This encapsulates the default field choice within the params module.
    pub fn apply_defaults(mut self) -> Self {
        <Self as WithSortDefaults>::apply_sort_defaults(
            &mut self.sort_by,
            &mut self.sort_order,
            SortField::Date,
        );
        self
    }
}

impl QuerySort<coaching_sessions::Column> for IndexParams {
    fn get_sort_column(&self) -> Option<coaching_sessions::Column> {
        self.sort_by.as_ref().map(|field| match field {
            SortField::Date => coaching_sessions::Column::Date,
            SortField::CreatedAt => coaching_sessions::Column::CreatedAt,
            SortField::UpdatedAt => coaching_sessions::Column::UpdatedAt,
        })
    }

    fn get_sort_order(&self) -> Option<Order> {
        self.sort_order.as_ref().map(|order| match order {
            SortOrder::Asc => Order::Asc,
            SortOrder::Desc => Order::Desc,
        })
    }
}

impl WithSortDefaults for IndexParams {
    type SortField = SortField;
}

/// Aggregation grouping for the counts endpoint.
///
/// v1 accepts only `month`. Invalid values are rejected by serde at the Axum
/// deserialization boundary with a 400 before reaching the handler.
#[derive(Debug, Clone, Copy, Deserialize, PartialEq, Eq, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum GroupByParam {
    Month,
}

/// Query parameters for GET `/users/{user_id}/coaching_sessions/counts`.
///
/// The `tz` field carries an IANA timezone identifier as a raw string; the
/// handler parses it via `chrono_tz::Tz::from_str` and returns
/// `WebErrorKind::InvalidTimezone` (400) if it does not match. Keeping the
/// type as `String` rather than `chrono_tz::Tz` here lets the handler
/// produce a structured discriminator response with the offending value
/// rather than Axum's default plain-text rejection.
#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct CountsByMonthParams {
    #[serde(skip)]
    pub(crate) user_id: Id,
    pub(crate) from_date: NaiveDate,
    pub(crate) to_date: NaiveDate,
    pub(crate) group_by: GroupByParam,
    pub(crate) tz: String,
    pub(crate) coaching_relationship_id: Option<Id>,
}

impl CountsByMonthParams {
    pub fn with_user_id(mut self, user_id: Id) -> Self {
        self.user_id = user_id;
        self
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // The v1 contract accepts only `group_by=month`. Anything else must fail
    // deserialization so axum rejects the request with 400 before the handler
    // runs. If the enum ever grows (e.g. `Week`), the handler's exhaustive
    // match catches the missing branch at compile time.
    #[test]
    fn group_by_param_accepts_month_and_rejects_other_values() {
        let month: GroupByParam = serde_json::from_str(r#""month""#).expect("month parses");
        assert_eq!(month, GroupByParam::Month);

        assert!(serde_json::from_str::<GroupByParam>(r#""week""#).is_err());
        assert!(serde_json::from_str::<GroupByParam>(r#""MONTH""#).is_err());
        assert!(serde_json::from_str::<GroupByParam>(r#""""#).is_err());
    }
}
