-- The built-in `user` role becomes the panel's Mods operator: it may manage mod
-- files, control the Minecraft container and create accounts that inherit its
-- own access. Existing databases receive the grants here; fresh databases get
-- the same defaults from the boot seeder (`src/seed.rs`).

INSERT INTO permissions (code, description) VALUES
    ('mods:upload', 'Upload mod files'),
    ('mods:delete', 'Delete mod files'),
    ('server:manage', 'Inspect, start, stop and restart the Minecraft server'),
    ('users:create', 'Create users with access not exceeding your own')
ON CONFLICT (code) DO NOTHING;

INSERT INTO role_permissions (role_id, permission_id)
SELECT r.id, p.id
FROM roles r
CROSS JOIN permissions p
WHERE r.name = 'user'
  AND p.code IN (
      'mods:upload', 'mods:delete', 'server:manage', 'users:create'
  )
ON CONFLICT DO NOTHING;
