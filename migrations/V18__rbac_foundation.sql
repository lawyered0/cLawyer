-- RBAC foundation: user identities/roles and per-matter membership mappings.

CREATE TABLE IF NOT EXISTS users (
    id TEXT PRIMARY KEY,
    display_name TEXT NOT NULL,
    role TEXT NOT NULL CHECK (role IN ('admin', 'attorney', 'staff', 'viewer')),
    is_active BOOLEAN NOT NULL DEFAULT TRUE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_users_role_active ON users(role, is_active);

CREATE TABLE IF NOT EXISTS matter_memberships (
    id UUID PRIMARY KEY DEFAULT gen_random_uuid(),
    matter_owner_user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    member_user_id TEXT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('owner', 'collaborator', 'viewer')),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (matter_owner_user_id, matter_id, member_user_id),
    FOREIGN KEY (matter_owner_user_id, matter_id)
        REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_matter_memberships_owner_matter
    ON matter_memberships(matter_owner_user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_memberships_member
    ON matter_memberships(member_user_id);
