use utoipa::Modify;
use utoipa::openapi::OpenApi;
use utoipa::openapi::security::{HttpAuthScheme, HttpBuilder, SecurityScheme};

use crate::app::HealthResponse;
use crate::auth::dto::{LoginRequest, MeResponse, TokenResponse};
use crate::error::ErrorResponse;
use crate::mods::dto::{ModFileActionDto, ModFileDto, UploadModForm};
use crate::rbac::dto::{
    AccessResponse, CreatePermissionRequest, CreateRoleRequest, PermissionDto,
    ReplacePermissionsRequest, RoleDto, RoleSummary, UpdateRoleRequest,
};
use crate::users::dto::{CreateUserRequest, UpdateUserRequest, UserDetail, UserSummary};

#[derive(utoipa::OpenApi)]
#[openapi(
    info(
        title = "Minecraft Mods API",
        version = "1.0.0",
        description = "A mod file directory behind JWT authentication and a dynamic permission \
                       system. The catalog is the listing of MODS_DIR: uploads, downloads and \
                       deletions are checked against per-role rules (mods:read, mods:download, \
                       mods:upload, mods:delete) and written to an audit trail."
    ),
    paths(
        crate::auth::routes::login,
        crate::auth::routes::me,
        crate::users::routes::list_users,
        crate::users::routes::create_user,
        crate::users::routes::get_user,
        crate::users::routes::update_user,
        crate::users::routes::delete_user,
        crate::users::routes::assign_role,
        crate::users::routes::revoke_role,
        crate::users::routes::grant_permission,
        crate::users::routes::revoke_permission,
        crate::rbac::routes::list_permissions,
        crate::rbac::routes::create_permission,
        crate::rbac::routes::delete_permission,
        crate::rbac::routes::list_roles,
        crate::rbac::routes::get_role,
        crate::rbac::routes::create_role,
        crate::rbac::routes::update_role,
        crate::rbac::routes::delete_role,
        crate::rbac::routes::replace_role_permissions,
        crate::mods::routes::list_mods,
        crate::mods::routes::upload_mod,
        crate::mods::routes::download_mod,
        crate::mods::routes::delete_mod,
        crate::mods::routes::file_history,
        crate::mods::routes::list_actions,
        crate::app::health
    ),
    components(schemas(
        ErrorResponse,
        LoginRequest,
        TokenResponse,
        MeResponse,
        UserSummary,
        UserDetail,
        CreateUserRequest,
        UpdateUserRequest,
        PermissionDto,
        RoleSummary,
        RoleDto,
        CreatePermissionRequest,
        CreateRoleRequest,
        UpdateRoleRequest,
        ReplacePermissionsRequest,
        AccessResponse,
        ModFileDto,
        ModFileActionDto,
        UploadModForm,
        HealthResponse
    )),
    tags(
        (name = "auth", description = "Login and the current user"),
        (name = "users", description = "Staff accounts and their access"),
        (name = "roles", description = "Roles and the permissions attached to them"),
        (name = "permissions", description = "The dynamic rule catalog"),
        (name = "mods", description = "Mod files on disk and the audit trail"),
        (name = "meta", description = "Health and service metadata")
    ),
    modifiers(&SecurityAddon)
)]
pub struct ApiDoc;

struct SecurityAddon;

impl Modify for SecurityAddon {
    fn modify(&self, openapi: &mut OpenApi) {
        if let Some(components) = openapi.components.as_mut() {
            components.add_security_scheme(
                "bearer_auth",
                SecurityScheme::Http(
                    HttpBuilder::new()
                        .scheme(HttpAuthScheme::Bearer)
                        .bearer_format("JWT")
                        .build(),
                ),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use utoipa::OpenApi as _;

    use super::ApiDoc;

    #[test]
    fn documents_every_route() {
        let document = ApiDoc::openapi();
        let paths = &document.paths.paths;

        for expected in [
            "/health",
            "/api/v1/auth/login",
            "/api/v1/auth/me",
            "/api/v1/users",
            "/api/v1/users/{id}/roles/{role_id}",
            "/api/v1/users/{id}/permissions/{permission_id}",
            "/api/v1/permissions",
            "/api/v1/roles",
            "/api/v1/roles/{id}/permissions",
            "/api/v1/mods",
            "/api/v1/mods/{file_name}",
            "/api/v1/mods/{file_name}/download",
            "/api/v1/mods/{file_name}/history",
            "/api/v1/mod-actions",
        ] {
            assert!(
                paths.contains_key(expected),
                "OpenAPI document is missing {expected}"
            );
        }

        let components = document.components.expect("components must exist");
        assert!(
            components.security_schemes.contains_key("bearer_auth"),
            "bearer_auth security scheme is missing"
        );

        let users = paths
            .get("/api/v1/users")
            .expect("/api/v1/users is missing");
        assert!(
            users.post.is_some(),
            "POST /api/v1/users (admin-only creation) is not documented"
        );

        let mods = paths.get("/api/v1/mods").expect("/api/v1/mods is missing");
        assert!(
            mods.post.is_some(),
            "POST /api/v1/mods (multipart upload) is not documented"
        );
    }
}
