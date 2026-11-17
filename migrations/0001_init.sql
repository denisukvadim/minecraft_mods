-- Initial schema: accounts, dynamic roles/permissions, and the audit trail.
-- Permission codes and the system roles are seeded at boot from
-- `PERMISSION_CATALOG` in `src/rbac/service.rs`, not by this migration.

CREATE TABLE users (
    id            uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    email         text NOT NULL UNIQUE,
    username      text NOT NULL UNIQUE,
    password_hash text NOT NULL,
    is_active     boolean NOT NULL DEFAULT true,
    created_at    timestamptz NOT NULL DEFAULT now(),
    updated_at    timestamptz NOT NULL DEFAULT now()
);

-- A permission is a row, not an enum: new rules can be created at runtime.
CREATE TABLE permissions (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    code        text NOT NULL UNIQUE,
    description text,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE roles (
    id          uuid PRIMARY KEY DEFAULT gen_random_uuid(),
    name        text NOT NULL UNIQUE,
    description text,
    -- System roles are seeded at boot and cannot be deleted.
    is_system   boolean NOT NULL DEFAULT false,
    created_at  timestamptz NOT NULL DEFAULT now()
);

CREATE TABLE role_permissions (
    role_id       uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    permission_id uuid NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    PRIMARY KEY (role_id, permission_id)
);

CREATE TABLE user_roles (
    user_id uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role_id uuid NOT NULL REFERENCES roles(id) ON DELETE CASCADE,
    PRIMARY KEY (user_id, role_id)
);

-- Direct grants: access that belongs to one user and survives role edits.
CREATE TABLE user_permissions (
    user_id       uuid NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    permission_id uuid NOT NULL REFERENCES permissions(id) ON DELETE CASCADE,
    granted_at    timestamptz NOT NULL DEFAULT now(),
    PRIMARY KEY (user_id, permission_id)
);

-- Audit trail: one row for every upload, download and delete. Identity
-- (email/username) is joined live from `users`, so deleting an account leaves
-- the trail with `user_id = NULL` rather than losing the history.
CREATE TABLE mod_file_actions (
    id         bigserial PRIMARY KEY,
    user_id    uuid REFERENCES users(id) ON DELETE SET NULL,
    action     text NOT NULL CHECK (action IN ('upload', 'download', 'delete')),
    file_name  text NOT NULL,
    created_at timestamptz NOT NULL DEFAULT now()
);

-- Reverse lookups: the primary keys already cover (user_id, ...) and (role_id, ...).
CREATE INDEX idx_role_permissions_permission ON role_permissions (permission_id);
CREATE INDEX idx_user_permissions_permission ON user_permissions (permission_id);
CREATE INDEX idx_user_roles_role ON user_roles (role_id);
CREATE INDEX idx_mod_file_actions_created_at ON mod_file_actions (created_at DESC, id DESC);
CREATE INDEX idx_mod_file_actions_user ON mod_file_actions (user_id);
CREATE INDEX idx_mod_file_actions_file ON mod_file_actions (file_name);
