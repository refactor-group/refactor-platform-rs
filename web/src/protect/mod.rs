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
use domain::{coaching_relationship, coaching_session, user as UserApi, Id};
use log::*;

/// Trait representing a single authorization rule.
///
/// Implementors answer **"is the authenticated user allowed to proceed?"**.
/// The rule receives:
/// * shared application state (`AppState`)
/// * the authenticated [`domain::users::Model`] (with populated `roles` field)
/// * any additional [`Id`] parameters supplied by the caller.
///
/// # Role-Based Authorization
///
/// Users have a `roles` field containing a vector of [`domain::user_roles::Model`] entries.
/// Each role has:
/// * `role`: The role type (e.g., `SuperAdmin`, `Admin`, `Coach`, etc.)
/// * `organization_id`: Optional organization scope (`None` for global roles like `SuperAdmin`)
///
/// Example:
/// ```rust,ignore
/// #[async_trait]
/// impl Check for UserIsAdmin {
///     async fn eval(&self, _app: &AppState, user: &domain::users::Model, args: Vec<Id>) -> bool {
///         // Check if user is a SuperAdmin (global access)
///         if user.roles.iter().any(|r|
///             r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none()
///         ) {
///             return true;
///         }
///
///         // Check for Admin role in specific organization
///         if let Some(org_id) = args.first() {
///             return user.roles.iter().any(|r|
///                 r.role == domain::users::Role::Admin && r.organization_id == Some(*org_id)
///             );
///         }
///
///         false
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
///         Predicate::new(UserIsAdmin, vec![org_id]),
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

/// Checks if the authenticated user is associated with the specified organization.
///
/// Returns `true` if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * User has any role in the specified organization
///
/// # Arguments
/// * `args[0]` - The organization ID to check
pub struct UserInOrganization;

#[async_trait]
impl Check for UserInOrganization {
    async fn eval(
        &self,
        app_state: &AppState,
        authenticated_user: &domain::users::Model,
        args: Vec<Id>,
    ) -> bool {
        // SuperAdmins have access to all organizations
        if authenticated_user
            .roles
            .iter()
            .any(|r| r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none())
        {
            return true;
        }

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

/// Checks if the authenticated user is NOT the user specified in args.
///
/// This is useful for preventing users from performing actions on themselves
/// (e.g., deleting their own account, removing their own admin privileges).
///
/// # Arguments
/// * `args[0]` - The user ID to check against
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

/// Checks if the authenticated user has admin privileges.
///
/// Returns `true` if:
/// * User is a SuperAdmin (has `SuperAdmin` role with `organization_id = NULL`), OR
/// * If an organization_id is provided in args, user has `Admin` role for that organization
///
/// # Arguments
/// * `args[0]` (optional) - The organization ID to check admin privileges for.
///   If not provided, only SuperAdmin check is performed.
pub struct UserIsAdmin;

#[async_trait]
impl Check for UserIsAdmin {
    async fn eval(
        &self,
        _app_state: &AppState,
        authenticated_user: &domain::users::Model,
        args: Vec<Id>,
    ) -> bool {
        // Check if user is a SuperAdmin (global access)
        if authenticated_user
            .roles
            .iter()
            .any(|r| r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none())
        {
            return true;
        }

        // If organization_id is provided, check for Admin role in that organization
        if let Some(organization_id) = args.first() {
            return authenticated_user.roles.iter().any(|r| {
                r.role == domain::users::Role::Admin && r.organization_id == Some(*organization_id)
            });
        }

        warn!(
            "UserIsAdmin check failed: no organization_id provided and user {} is not SuperAdmin",
            authenticated_user.id
        );
        false
    }
}
/// Checks if the authenticated user is a SuperAdmin (global admin).
///
/// Returns `true` if user has the `SuperAdmin` role with `organization_id = NULL`.
///
/// **Note:** This check is deprecated in favor of using `UserIsAdmin` without
/// an organization_id argument, as that also checks for SuperAdmin status.
/// This struct is kept for explicit SuperAdmin-only checks if needed.
#[allow(dead_code)]
pub struct UserIsSuperAdmin;

#[async_trait]
impl Check for UserIsSuperAdmin {
    async fn eval(
        &self,
        _app_state: &AppState,
        authenticated_user: &domain::users::Model,
        _args: Vec<Id>,
    ) -> bool {
        authenticated_user
            .roles
            .iter()
            .any(|r| r.role == domain::users::Role::SuperAdmin && r.organization_id.is_none())
    }
}

/// Checks if the authenticated user can access a specific coaching session.
///
/// Returns `true` if the user is either the coach or coachee in the coaching
/// relationship associated with the session.
///
/// # Arguments
/// * `args[0]` - The coaching session ID to check access for
pub struct UserCanAccessCoachingSession;

#[async_trait]
impl Check for UserCanAccessCoachingSession {
    async fn eval(
        &self,
        app_state: &AppState,
        authenticated_user: &domain::users::Model,
        args: Vec<Id>,
    ) -> bool {
        let coaching_session_id = args[0];

        // Get the coaching session
        let coaching_session = match coaching_session::find_by_id(
            app_state.db_conn_ref(),
            coaching_session_id,
        )
        .await
        {
            Ok(session) => session,
            Err(e) => {
                error!("Error finding coaching session {coaching_session_id}: {e:?}");
                return false;
            }
        };

        // Get the coaching relationship
        let coaching_relationship = match coaching_relationship::find_by_id(
            app_state.db_conn_ref(),
            coaching_session.coaching_relationship_id,
        )
        .await
        {
            Ok(relationship) => relationship,
            Err(e) => {
                error!(
                    "Error finding coaching relationship {}: {e:?}",
                    coaching_session.coaching_relationship_id
                );
                return false;
            }
        };

        // Check if user is coach or coachee
        coaching_relationship.coach_id == authenticated_user.id
            || coaching_relationship.coachee_id == authenticated_user.id
    }
}
