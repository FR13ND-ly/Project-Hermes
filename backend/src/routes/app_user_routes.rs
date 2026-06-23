use axum::{
    routing::{post, get, delete},
    Router,
    middleware::from_fn_with_state,
    Extension,
};
use crate::app_state::AppState;
use crate::controllers::app_user_controller;
use crate::middlewares::permission_middleware::{check_permission, RequiredPermission};

pub fn routes(state: AppState) -> Router {
    // Public BaaS end-user auth endpoints (no dashboard auth; keyed by service id).
    let public_routes = Router::new()
        .route("/baas/:id/auth/register", post(app_user_controller::register_public_user))
        .route("/baas/:id/auth/login", post(app_user_controller::login_public_user))
        .route("/baas/:id/auth/refresh", post(app_user_controller::refresh_app_token))
        .route("/baas/:id/auth/logout", post(app_user_controller::logout_app_user))
        .route("/baas/:id/auth/verify-token", post(app_user_controller::verify_app_token))
        .route("/baas/:id/auth/verify-key", post(app_user_controller::verify_app_key));

    // Create / delete a standalone BaaS service (project resource).
    let manage_router = Router::new()
        .route("/baas", post(app_user_controller::create_baas_service))
        .route("/baas/:id", delete(app_user_controller::delete_baas_service))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:deploy")));

    let list_router = Router::new()
        .route("/projects/:project_id/baas", get(app_user_controller::list_project_baas_services))
        .route("/baas/:id", get(app_user_controller::get_baas_service))
        .route("/baas/:id/users", get(app_user_controller::list_app_users_with_roles))
        .route("/baas/:id/auth-config", get(app_user_controller::get_app_auth_config))
        .route("/baas/:id/api-keys", get(app_user_controller::list_app_api_keys))
        .route("/baas/:id/auth/integration", get(app_user_controller::get_auth_integration))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:read")));

    let roles_router = Router::new()
        .route("/baas/:id/users/roles", post(app_user_controller::assign_user_role_to_app).delete(app_user_controller::remove_user_role_from_app))
        .route("/baas/:id/users/:user_id/status", post(app_user_controller::update_app_user_status))
        .route("/baas/:id/users/:user_id/reset-password", post(app_user_controller::reset_app_user_password))
        .route("/baas/:id/auth-config", post(app_user_controller::update_app_auth_config))
        .route("/baas/:id/auth/rotate-secret", post(app_user_controller::rotate_auth_secret))
        .route("/baas/:id/api-keys", post(app_user_controller::create_app_api_key))
        .route("/baas/:id/api-keys/:key_id", delete(app_user_controller::delete_app_api_key))
        .layer(from_fn_with_state(state.clone(), check_permission))
        .layer(Extension(RequiredPermission("app:update")));

    Router::new()
        .merge(public_routes)
        .merge(manage_router)
        .merge(list_router)
        .merge(roles_router)
        .with_state(state)
}
