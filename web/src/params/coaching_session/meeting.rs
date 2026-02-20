use domain::provider::Provider;
use serde::Deserialize;
use utoipa::{IntoParams, ToSchema};

#[derive(Debug, Deserialize, IntoParams, ToSchema)]
pub(crate) struct CreateParams {
    pub(crate) provider: Provider,
}
