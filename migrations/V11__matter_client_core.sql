-- Matter/client core schema (Phase 2 Task 1)
-- Adds normalized clients + DB-backed matter/task/note records.

CREATE EXTENSION IF NOT EXISTS pg_trgm;

CREATE TABLE IF NOT EXISTS clients (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    name TEXT NOT NULL,
    name_normalized TEXT NOT NULL,
    client_type TEXT NOT NULL CHECK (client_type IN ('individual', 'entity')),
    email TEXT,
    phone TEXT,
    address TEXT,
    notes TEXT,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (user_id, name_normalized)
);

CREATE INDEX IF NOT EXISTS idx_clients_user ON clients(user_id);
CREATE INDEX IF NOT EXISTS idx_clients_user_name_normalized ON clients(user_id, name_normalized);
CREATE INDEX IF NOT EXISTS idx_clients_name_trgm ON clients USING GIN (name_normalized gin_trgm_ops);

CREATE TABLE IF NOT EXISTS matters (
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    client_id UUID NOT NULL REFERENCES clients(id) ON DELETE RESTRICT,
    status TEXT NOT NULL CHECK (status IN ('intake', 'active', 'pending', 'closed', 'archived')),
    stage TEXT,
    practice_area TEXT,
    jurisdiction TEXT,
    opened_at TIMESTAMPTZ,
    closed_at TIMESTAMPTZ,
    assigned_to JSONB NOT NULL DEFAULT '[]'::jsonb,
    custom_fields JSONB NOT NULL DEFAULT '{}'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (user_id, matter_id)
);

CREATE INDEX IF NOT EXISTS idx_matters_user_status ON matters(user_id, status);
CREATE INDEX IF NOT EXISTS idx_matters_client ON matters(client_id);

CREATE TABLE IF NOT EXISTS matter_tasks (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    title TEXT NOT NULL,
    description TEXT,
    status TEXT NOT NULL CHECK (status IN ('todo', 'in_progress', 'done', 'blocked', 'cancelled')),
    assignee TEXT,
    due_at TIMESTAMPTZ,
    blocked_by JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_matter_tasks_user_matter ON matter_tasks(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_tasks_user_status ON matter_tasks(user_id, status);

CREATE TABLE IF NOT EXISTS matter_notes (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    author TEXT NOT NULL,
    body TEXT NOT NULL,
    pinned BOOL NOT NULL DEFAULT FALSE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE
);

CREATE INDEX IF NOT EXISTS idx_matter_notes_user_matter ON matter_notes(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_notes_user_created ON matter_notes(user_id, created_at DESC);
