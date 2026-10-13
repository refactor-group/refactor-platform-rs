//! Identity updates against a real SQL engine: they must never disturb the stored tokens.

use chrono::Utc;
use entity::meeting_provider::Provider as MeetingProvider;
use entity::oauth_connections::ActiveModel;
use entity_api::oauth_connection as ConnectionApi;
use sea_orm::{ActiveModelTrait, Set};

use super::*;
use crate::test_utils::sqlite::{database, seed_user, within_time_limit};

/// Updating identity rewrites only the account id and email of the named connection.
#[tokio::test]
async fn updating_identity_leaves_tokens_and_other_connections_alone() -> Result<(), Error> {
    within_time_limit(async {
        let db = database().await;
        let user_id = seed_user(&db).await;
        let now = Utc::now();

        let connection = |provider: MeetingProvider| ActiveModel {
            id: Set(Id::new_v4()),
            user_id: Set(user_id),
            provider: Set(provider),
            external_account_id: Set(None),
            external_email: Set(Some("old@example.com".to_string())),
            access_token: Set(format!("encrypted-access-{provider}")),
            refresh_token: Set(Some(format!("encrypted-refresh-{provider}"))),
            token_expires_at: Set(None),
            token_type: Set("Bearer".to_string()),
            scopes: Set("openid email".to_string()),
            created_at: Set(now.into()),
            updated_at: Set(now.into()),
        };
        let google = connection(MeetingProvider::Google)
            .insert(&db)
            .await
            .expect("the google connection is seeded");
        let zoom = connection(MeetingProvider::Zoom)
            .insert(&db)
            .await
            .expect("the zoom connection is seeded");

        ConnectionApi::update_identity(
            &db,
            google.id,
            Some("google-123".to_string()),
            Some("coach@example.com".to_string()),
        )
        .await?;

        let google_after =
            ConnectionApi::get_by_user_and_provider(&db, user_id, MeetingProvider::Google).await?;
        assert_eq!(
            google_after.external_account_id.as_deref(),
            Some("google-123")
        );
        assert_eq!(
            google_after.external_email.as_deref(),
            Some("coach@example.com")
        );
        assert_eq!(google_after.access_token, google.access_token);
        assert_eq!(google_after.refresh_token, google.refresh_token);
        assert_eq!(google_after.scopes, google.scopes);

        let zoom_after =
            ConnectionApi::get_by_user_and_provider(&db, user_id, MeetingProvider::Zoom).await?;
        assert_eq!(zoom_after.external_account_id, None);
        assert_eq!(
            zoom_after.external_email.as_deref(),
            Some("old@example.com")
        );
        assert_eq!(zoom_after.access_token, zoom.access_token);

        Ok::<(), Error>(())
    })
    .await
}
