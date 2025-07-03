//! This module provides protection mechanisms for various resources in the web application.
//!
//! It includes submodules for authorizing access to resources. Each submodule contains the necessary logic to protect
//! the corresponding resources, ensuring that only authorized users can access or modify them.
//!
//! The protection mechanisms are designed to be flexible and extensible, allowing for the addition
//! of new resources and protection strategies as needed. By organizing the protection logic into
//! separate submodules, we can maintain a clear and modular structure, making the codebase easier
//! to understand and maintain.

pub(crate) mod actions;
pub(crate) mod agreements;
pub(crate) mod coaching_relationships;
pub(crate) mod coaching_sessions;
pub(crate) mod jwt;
pub(crate) mod notes;
pub(crate) mod organizations;
pub(crate) mod overarching_goals;
pub(crate) mod users;

use crate::AppState;
use async_trait::async_trait;
use axum::{extract::Request, http::StatusCode, middleware::Next, response::IntoResponse};
use domain::{user as UserApi, Id};
use log::*;

/// Trait representing a single authorization rule.
///
/// Implementors answer **“is the authenticated user allowed to proceed?”**.
/// The rule receives:
/// * shared application state (`AppState`)
/// * the authenticated [`domain::users::Model`]
/// * any additional [`Id`] parameters supplied by the caller.
///
/// Example:
/// ```rust,ignore
/// #[async_trait]
/// impl Check for UserIsAdmin {
///     async fn eval(&self, _app: &AppState, user: &domain::users::Model, _args: Vec<Id>) -> bool {
///         user.role == domain::users::Role::Admin
///     }
/// }
/// ```
#[async_trait]
pub trait Check: Send + Sync {
    async fn eval(&self, app: &AppState, user: &domain::users::Model, args: Vec<Id>) -> bool;
}

/// Pairs a [`Check`] implementation with the concrete arguments that the rule
/// should receive when evaluated.
///
/// Most callers will create predicates with the convenience constructor
/// [`Predicate::new`]:
/// ```rust,ignore
/// let checks = vec![
///     Predicate::new(UserInOrganization, vec![org_id]),
///     Predicate::new(UserIsAdmin, vec![]),
/// ];
/// ```
/// The vector of predicates can then be passed to [`authorize`] middleware.
pub(crate) struct Predicate {
    predicate: Box<dyn Check>,
    args: Vec<Id>,
}

impl Predicate {
    pub(crate) fn new<C: Check + 'static>(predicate: C, args: Vec<Id>) -> Self {
        Self {
            predicate: Box::new(predicate),
            args,
        }
    }

    pub(crate) async fn check(&self, app_state: &AppState, user: &domain::users::Model) -> bool {
        self.predicate
            .eval(app_state, user, self.args.clone())
            .await
    }
}

/// Axum middleware that enforces one or more [`Predicate`]s.
///
/// Each predicate is evaluated in the order supplied; if any rule returns
/// `false` the request is aborted with **403 FORBIDDEN**.  When all rules
/// pass the wrapped handler (`next`) is executed.
///
/// Typical usage inside a helper function in the `protect` namespace:
/// ```rust,ignore
/// pub(crate) async fn index(
///     State(app_state): State<AppState>,
///     AuthenticatedUser(user): AuthenticatedUser,
///     Path(org_id): Path<Id>,
///     request: Request,
///     next: Next,
/// ) -> impl IntoResponse {
///     let checks = vec![
///         Predicate::new(UserInOrganization, vec![org_id]),
///         Predicate::new(UserIsAdmin, vec![]),
///     ];
///     authorize(&app_state, user, request, next, checks).await
/// }
/// ```
pub(crate) async fn authorize(
    app_state: &AppState,
    authenticated_user: domain::users::Model,
    request: Request,
    next: Next,
    checks: Vec<Predicate>,
) -> impl IntoResponse {
    for check in checks {
        if !check.check(app_state, &authenticated_user).await {
            return (StatusCode::FORBIDDEN, "FORBIDDEN").into_response();
        }
    }
    next.run(request).await
}

pub struct UserInOrganization;

#[async_trait]
impl Check for UserInOrganization {
    async fn eval(
        &self,
        app_state: &AppState,
        authenticated_user: &domain::users::Model,
        args: Vec<Id>,
    ) -> bool {
        let organization_id = args[0];
        match UserApi::find_by_organization(app_state.db_conn_ref(), organization_id).await {
            Ok(users) => users.iter().any(|user| user.id == authenticated_user.id),
            Err(_) => {
                error!("Organization not found with ID {organization_id:?}");
                false
            }
        }
    }
}

pub struct UserIsNotSelf;

#[async_trait]
impl Check for UserIsNotSelf {
    async fn eval(
        &self,
        _app_state: &AppState,
        authenticated_user: &domain::users::Model,
        args: Vec<Id>,
    ) -> bool {
        let user_id = args[0];
        authenticated_user.id != user_id
    }
}

pub struct UserIsAdmin;

#[async_trait]
impl Check for UserIsAdmin {
    async fn eval(
        &self,
        _app_state: &AppState,
        authenticated_user: &domain::users::Model,
        _args: Vec<Id>,
    ) -> bool {
        authenticated_user.role == domain::users::Role::Admin
    }
}
