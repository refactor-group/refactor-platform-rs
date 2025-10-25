use chrono::NaiveDate;
use serde::Deserialize;
use utoipa::IntoParams;

use crate::params::coaching_session::SortField;
use crate::params::sort::SortOrder;
use domain::Id;

#[derive(Debug, Deserialize, IntoParams)]
pub(crate) struct IndexParams {
    #[serde(skip)]
    pub(crate) user_id: Id,
    pub(crate) from_date: Option<NaiveDate>,
    pub(crate) to_date: Option<NaiveDate>,
    pub(crate) sort_by: Option<SortField>,
    pub(crate) sort_order: Option<SortOrder>,
}

impl IndexParams {
    pub fn new(user_id: Id) -> Self {
        Self {
            user_id,
            from_date: None,
            to_date: None,
            sort_by: None,
            sort_order: None,
        }
    }

    pub fn with_filters(
        mut self,
        from_date: Option<NaiveDate>,
        to_date: Option<NaiveDate>,
        sort_by: Option<SortField>,
        sort_order: Option<SortOrder>,
    ) -> Self {
        self.from_date = from_date;
        self.to_date = to_date;
        self.sort_by = sort_by;
        self.sort_order = sort_order;
        self
    }
}
