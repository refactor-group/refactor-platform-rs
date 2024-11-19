pub(crate) mod coaching_relationships;

#[macro_export]
macro_rules! protected_resource {
    ($protect:expr, $alternative:expr) => {{
        use axum::{
            extract::Request,
            middleware::{from_fn, Next},
            response::IntoResponse,
        };
        use entity_api::user::AuthSession;

        from_fn(
            |auth_session: AuthSession, req: Request, next: Next| async move {
                match $protect(auth_session, req).await {
                    Ok(req) => next.run(req).await,
                    Err(_) => $alternative.into_response(),
                }
            },
        )
    }};
}
