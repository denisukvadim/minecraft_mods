-- The built-in `user` role may also read the audit trail: Mods operators need
-- to see who uploaded, downloaded or deleted a file, without gaining the
-- access-management screens. Fresh databases get the same default from the
-- boot seeder (`src/seed.rs`).

INSERT INTO permissions (code, description) VALUES
    ('audit:read', 'View the audit trail')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
CROSS JOIN permissions p
WHERE r.name = 'user'
  AND p.code = 'audit:read'
ON CONFLICT DO NOTHING;
