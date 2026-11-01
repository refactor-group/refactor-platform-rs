use chrono::{DateTime, NaiveDate, NaiveDateTime};
use sea_orm::Order;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

use crate::error::WebErrorKind;
use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use crate::params::WithSortDefaults;
use crate::Error;
use domain::coaching_session::{IncludeOptions, NearestWindow};
use domain::{coaching_sessions, Id, QuerySort};

/// Most sessions `/nearest` returns on either side of `around`.
pub(crate) const MAX_NEAREST_PER_SIDE: u64 = 10;

/// Related resources that can be batch-loaded with coaching sessions.
///
/// Used in `?include=` query parameter to eliminate N+1 queries. Supports
/// comma-separated values: `?include=relationship,organization,goal,agreements`
///
/// Maps to `entity_api::coaching_session::IncludeOptions` for database queries.
#[derive(Debug, Clone, Deserialize, PartialEq, Eq, Hash, ToSchema)]
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
    /// Include the session's series id and its latest session date
    Series,
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
    /// Optional: filter sessions to relationships in this organization.
    ///
    /// Omitted = unscoped, matching the pre-multi-org behavior.
    #[serde(default)]
    pub(crate) organization_id: Option<Id>,
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

/// The batch-loads the `include` values ask for.
pub(crate) fn include_options(include: &[IncludeParam]) -> IncludeOptions {
    IncludeOptions {
        relationship: include.contains(&IncludeParam::Relationship),
        organization: include.contains(&IncludeParam::Organization),
        goal: include.contains(&IncludeParam::Goal),
        agreements: include.contains(&IncludeParam::Agreements),
        topics: include.contains(&IncludeParam::Topics),
        series: include.contains(&IncludeParam::Series),
    }
}

/// Query parameters for GET `/users/{user_id}/coaching_sessions/nearest`.
///
/// Every window field arrives as a string so a missing or malformed value gets the
/// structured `invalid_nearest_window` 400 instead of axum's plain-text rejection.
#[derive(Debug, Deserialize)]
pub(crate) struct NearestParams {
    pub(crate) coaching_relationship_id: Option<String>,
    /// UTC with no offset (the form a session's `date` takes), or RFC 3339 with one.
    pub(crate) around: Option<String>,
    pub(crate) before: Option<String>,
    pub(crate) after: Option<String>,
    #[serde(default, deserialize_with = "deserialize_comma_separated")]
    pub(crate) include: Vec<IncludeParam>,
}

impl NearestParams {
    /// Validate into the window to query. Every rejection is `invalid_nearest_window`.
    pub(crate) fn compile(&self) -> Result<NearestWindow, Error> {
        let coaching_relationship_id =
            required("coaching_relationship_id", &self.coaching_relationship_id)?
                .parse::<Id>()
                .map_err(|_| invalid_window("`coaching_relationship_id` must be a UUID."))?;
        let around = parse_around(required("around", &self.around)?).ok_or_else(|| {
            invalid_window(
                "`around` must be a datetime such as 2026-10-30T14:00:00 (UTC) or 2026-10-30T09:00:00-05:00.",
            )
        })?;

        Ok(NearestWindow {
            coaching_relationship_id,
            around,
            before: per_side_count("before", &self.before)?,
            after: per_side_count("after", &self.after)?,
        })
    }
}

fn invalid_window(message: &str) -> Error {
    Error::Web(WebErrorKind::InvalidParam {
        error: "invalid_nearest_window",
        message: message.to_string(),
    })
}

fn required<'a>(name: &str, value: &'a Option<String>) -> Result<&'a str, Error> {
    value
        .as_deref()
        .ok_or_else(|| invalid_window(&format!("`{name}` is required.")))
}

/// An offset is converted to UTC; a bare datetime is already UTC.
fn parse_around(raw: &str) -> Option<NaiveDateTime> {
    DateTime::parse_from_rfc3339(raw)
        .map(|at| at.naive_utc())
        .or_else(|_| NaiveDateTime::parse_from_str(raw, "%Y-%m-%dT%H:%M:%S%.f"))
        .ok()
}

fn per_side_count(name: &str, value: &Option<String>) -> Result<u64, Error> {
    required(name, value)?
        .parse::<u64>()
        .ok()
        .filter(|count| *count <= MAX_NEAREST_PER_SIDE)
        .ok_or_else(|| {
            invalid_window(&format!(
                "`{name}` must be a whole number from 0 to {MAX_NEAREST_PER_SIDE}."
            ))
        })
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
    /// Optional: restrict the counts to relationships in this organization.
    #[serde(default)]
    pub(crate) organization_id: Option<Id>,
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

    #[test]
    fn include_param_parses_series() {
        let uri: axum::http::Uri = "/?include=series".parse().unwrap();
        let axum::extract::Query(params) =
            axum::extract::Query::<IndexParams>::try_from_uri(&uri).expect("include=series parses");
        assert_eq!(params.include, vec![IncludeParam::Series]);
    }

    const RELATIONSHIP_ID: &str = "7b16a3b2-a709-4068-95d2-c1600db57cae";

    fn nearest(query: &str) -> NearestParams {
        let uri: axum::http::Uri = format!("/?{query}").parse().unwrap();
        axum::extract::Query::<NearestParams>::try_from_uri(&uri)
            .expect("the query deserializes")
            .0
    }

    /// A full query with `field` replaced by `value`, or left out when `value` is `None`.
    fn query_with(field: &str, value: Option<&str>) -> String {
        [
            ("coaching_relationship_id", RELATIONSHIP_ID),
            ("around", "2026-10-30T14:00:00"),
            ("before", "2"),
            ("after", "3"),
        ]
        .into_iter()
        .filter_map(|(name, default)| match name == field {
            true => value.map(|v| format!("{name}={v}")),
            false => Some(format!("{name}={default}")),
        })
        .collect::<Vec<_>>()
        .join("&")
    }

    fn valid(around: &str) -> String {
        query_with("around", Some(around))
    }

    fn utc(hour: u32, minute: u32) -> NaiveDateTime {
        NaiveDate::from_ymd_opt(2026, 10, 30)
            .unwrap()
            .and_hms_opt(hour, minute, 0)
            .unwrap()
    }

    /// The `invalid_nearest_window` message, or a panic for any other outcome.
    fn rejection(params: NearestParams) -> String {
        match params.compile() {
            Err(Error::Web(WebErrorKind::InvalidParam {
                error: "invalid_nearest_window",
                message,
            })) => message,
            other => panic!("expected invalid_nearest_window, got {other:?}"),
        }
    }

    #[test]
    fn a_full_query_compiles_into_the_window() {
        let window = nearest(&valid("2026-10-30T14:00:00"))
            .compile()
            .expect("compiles");
        assert_eq!(
            window,
            NearestWindow {
                coaching_relationship_id: RELATIONSHIP_ID.parse().unwrap(),
                around: utc(14, 0),
                before: 2,
                after: 3,
            }
        );
    }

    #[test]
    fn around_without_an_offset_is_read_as_utc_like_a_session_date() {
        let session_date = utc(14, 0).format("%Y-%m-%dT%H:%M:%S").to_string();
        let window = nearest(&valid(&session_date)).compile().expect("compiles");
        assert_eq!(window.around, utc(14, 0));
    }

    #[test]
    fn around_with_an_offset_is_converted_to_utc() {
        for around in [
            "2026-10-30T14:00:00Z",
            "2026-10-30T09:00:00-05:00",
            "2026-10-30T19:30:00%2B05:30",
        ] {
            let window = nearest(&valid(around)).compile().expect("compiles");
            assert_eq!(window.around, utc(14, 0), "{around}");
        }
    }

    #[test]
    fn around_keeps_fractional_seconds() {
        let window = nearest(&valid("2026-10-30T14:00:00.250"))
            .compile()
            .expect("compiles");
        assert_eq!(
            window.around,
            utc(14, 0) + chrono::TimeDelta::milliseconds(250)
        );
    }

    /// An unencoded `+` decodes to a space, so a `+hh:mm` offset must be sent as `%2B`.
    #[test]
    fn around_with_an_unencoded_plus_is_rejected() {
        assert!(rejection(nearest(&valid("2026-10-30T19:30:00+05:30"))).contains("`around`"));
    }

    #[test]
    fn around_that_is_not_a_datetime_is_rejected() {
        for around in ["2026-10-30", "14:00", "yesterday", ""] {
            assert!(
                rejection(nearest(&valid(around))).contains("`around`"),
                "{around}"
            );
        }
    }

    #[test]
    fn each_missing_field_is_named() {
        for field in ["coaching_relationship_id", "around", "before", "after"] {
            let query = query_with(field, None);
            assert_eq!(
                rejection(nearest(&query)),
                format!("`{field}` is required.")
            );
        }
    }

    #[test]
    fn a_malformed_relationship_id_is_rejected() {
        let query = query_with("coaching_relationship_id", Some("nope"));
        assert!(rejection(nearest(&query)).contains("`coaching_relationship_id`"));
    }

    #[test]
    fn counts_from_zero_to_ten_are_accepted() {
        let window = nearest(&query_with("before", Some("0")))
            .compile()
            .expect("compiles");
        assert_eq!(window.before, 0);
        let window = nearest(&query_with("after", Some("10")))
            .compile()
            .expect("compiles");
        assert_eq!(window.after, MAX_NEAREST_PER_SIDE);
    }

    #[test]
    fn counts_outside_zero_to_ten_or_not_whole_numbers_are_rejected() {
        for (field, value) in [
            ("before", "11"),
            ("after", "-1"),
            ("before", "two"),
            ("after", "2.5"),
        ] {
            assert_eq!(
                rejection(nearest(&query_with(field, Some(value)))),
                format!("`{field}` must be a whole number from 0 to 10."),
                "{field}={value}"
            );
        }
    }

    #[test]
    fn every_include_value_maps_to_its_own_batch_load() {
        let params = nearest(&format!(
            "{}&include=relationship,organization,goal,agreements,topics,series",
            valid("2026-10-30T14:00:00")
        ));
        let includes = include_options(&params.include);
        assert!(
            includes.relationship
                && includes.organization
                && includes.goal
                && includes.agreements
                && includes.topics
                && includes.series,
            "{includes:?}"
        );
    }

    #[test]
    fn include_relationship_maps_to_the_relationship_batch_load() {
        let params = nearest(&format!(
            "{}&include=relationship",
            valid("2026-10-30T14:00:00")
        ));
        let includes = include_options(&params.include);
        assert!(includes.relationship);
        assert!(!includes.organization && !includes.goal && !includes.agreements);
    }
}
