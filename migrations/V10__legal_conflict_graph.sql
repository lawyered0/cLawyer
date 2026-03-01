-- Legal conflict graph schema (v1)
-- Adds party graph + matter-party links + conflict clearance records.
-- Recursive relationship traversal is intentionally deferred in this migration.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE IF NOT EXISTS parties (
    id UUID PRIMARY KEY,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL UNIQUE,
    party_type TEXT NOT NULL CHECK (party_type IN ('individual', 'entity')),
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_parties_name_normalized ON parties(name_normalized);
CREATE INDEX IF NOT EXISTS idx_parties_name_trgm ON parties USING GIN (name_normalized gin_trgm_ops);

CREATE TABLE IF NOT EXISTS party_aliases (
    id UUID PRIMARY KEY,
    party_id UUID NOT NULL REFERENCES parties(id) ON DELETE CASCADE,
    alias TEXT NOT NULL,
    alias_normalized TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (party_id, alias_normalized)
);

CREATE INDEX IF NOT EXISTS idx_party_aliases_party_id ON party_aliases(party_id);
CREATE INDEX IF NOT EXISTS idx_party_aliases_alias_normalized ON party_aliases(alias_normalized);
CREATE INDEX IF NOT EXISTS idx_party_aliases_alias_trgm ON party_aliases USING GIN (alias_normalized gin_trgm_ops);

CREATE TABLE IF NOT EXISTS party_relationships (
    id UUID PRIMARY KEY,
    parent_id UUID NOT NULL REFERENCES parties(id) ON DELETE CASCADE,
    child_id UUID NOT NULL REFERENCES parties(id) ON DELETE CASCADE,
    kind TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (parent_id, child_id, kind)
);

CREATE INDEX IF NOT EXISTS idx_party_relationships_parent_id ON party_relationships(parent_id);
CREATE INDEX IF NOT EXISTS idx_party_relationships_child_id ON party_relationships(child_id);

CREATE TABLE IF NOT EXISTS matter_parties (
    id UUID PRIMARY KEY,
    matter_id TEXT NOT NULL,
    party_id UUID NOT NULL REFERENCES parties(id) ON DELETE CASCADE,
    role TEXT NOT NULL CHECK (role IN ('client', 'adverse', 'related', 'witness')),
    opened_at TIMESTAMPTZ,
    closed_at TIMESTAMPTZ,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (matter_id, party_id, role)
);

CREATE INDEX IF NOT EXISTS idx_matter_parties_party_id ON matter_parties(party_id);
CREATE INDEX IF NOT EXISTS idx_matter_parties_matter_id ON matter_parties(matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_parties_role_closed_at ON matter_parties(role, closed_at);

CREATE TABLE IF NOT EXISTS conflict_clearances (
    id UUID PRIMARY KEY,
    matter_id TEXT NOT NULL,
    checked_by TEXT NOT NULL,
    cleared_by TEXT,
    decision TEXT NOT NULL CHECK (decision IN ('clear', 'waived', 'declined')),
    note TEXT,
    hits_json JSONB NOT NULL,
    hit_count INTEGER NOT NULL DEFAULT 0,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX IF NOT EXISTS idx_conflict_clearances_matter_created_at
    ON conflict_clearances(matter_id, created_at DESC);
