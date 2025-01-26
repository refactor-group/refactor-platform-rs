use super::error::Error;
use entity::coaching_sessions::Model;
use entity_api::{coaching_relationship, coaching_session, organization};
use sea_orm::DatabaseConnection;
use std::env;

pub async fn create(
    db: &DatabaseConnection,
    coaching_session_model: Model,
) -> Result<Model, Error> {
    let coaching_relationship =
        coaching_relationship::find_by_id(db, coaching_session_model.coaching_relationship_id)
            .await?
            .ok_or_else(|| Error::new(None).entity())?;

    let organization = organization::find_by_id(db, coaching_relationship.organization_id)
        .await?
        .ok_or_else(|| Error::new(None).entity())?;

    let document_name = format!(
        "{}.{}.{}",
        organization.slug, coaching_relationship.slug, coaching_session_model.date
    );
    let tip_tap_url =
        env::var("TIP_TAP_URL").map_err(|err| Error::new(Some(Box::new(err))).other())?;

    let full_url = format!("{}//api/documents/{}", tip_tap_url, document_name);

    let client = tip_tap_client().await?;

    let res = client.post(full_url).send().await?;

    if res.status().is_success() || res.status().as_u16() == 409 {
        Ok(coaching_session::create(db, coaching_session_model).await?)
    } else {
        Err(Error::new(None).network())
    }
}

pub async fn find_by_id(db: &DatabaseConnection, id: entity::Id) -> Result<Option<Model>, Error> {
    Ok(coaching_session::find_by_id(db, id).await?)
}

pub async fn find_by_id_with_coaching_relationship(
    db: &DatabaseConnection,
    id: entity::Id,
) -> Result<(Model, entity::coaching_relationships::Model), Error> {
    Ok(coaching_session::find_by_id_with_coaching_relationship(db, id).await?)
}

pub async fn find_by(
    db: &DatabaseConnection,
    params: std::collections::HashMap<String, String>,
) -> Result<Vec<Model>, Error> {
    Ok(coaching_session::find_by(db, params).await?)
}

async fn tip_tap_client() -> Result<reqwest::Client, Error> {
    let headers = build_auth_headers().await?;

    Ok(reqwest::Client::builder()
        .default_headers(headers)
        .build()?)
}

async fn build_auth_headers() -> Result<reqwest::header::HeaderMap, Error> {
    let auth_key =
        env::var("TIP_TAP_AUTH_KEY").map_err(|err| Error::new(Some(Box::new(err))).other())?;
    let mut headers = reqwest::header::HeaderMap::new();
    let mut auth_value = reqwest::header::HeaderValue::from_str(&auth_key)
        .map_err(|err| Error::new(Some(Box::new(err))).other())?;
    auth_value.set_sensitive(true);
    headers.insert(reqwest::header::AUTHORIZATION, auth_value);
    Ok(headers)
}
