-- Matter document management schema (Phase 2 Task 3)
-- Adds matter-linked document records, version history, and templates.

CREATE TABLE IF NOT EXISTS matter_documents (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT NOT NULL,
    memory_document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    display_name TEXT NOT NULL,
    category TEXT NOT NULL CHECK (category IN (
        'pleading',
        'correspondence',
        'contract',
        'filing',
        'evidence',
        'internal'
    )),
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE,
    UNIQUE (user_id, matter_id, memory_document_id)
);

CREATE INDEX IF NOT EXISTS idx_matter_documents_user_matter
    ON matter_documents(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_matter_documents_memory_document
    ON matter_documents(memory_document_id);

CREATE TABLE IF NOT EXISTS document_versions (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_document_id UUID NOT NULL REFERENCES matter_documents(id) ON DELETE CASCADE,
    version_number INTEGER NOT NULL CHECK (version_number > 0),
    label TEXT NOT NULL,
    memory_document_id UUID NOT NULL REFERENCES memory_documents(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    UNIQUE (matter_document_id, version_number)
);

CREATE INDEX IF NOT EXISTS idx_document_versions_user_matter_document
    ON document_versions(user_id, matter_document_id);
CREATE INDEX IF NOT EXISTS idx_document_versions_memory_document
    ON document_versions(memory_document_id);

CREATE TABLE IF NOT EXISTS document_templates (
    id UUID PRIMARY KEY,
    user_id TEXT NOT NULL,
    matter_id TEXT,
    name TEXT NOT NULL,
    body TEXT NOT NULL,
    variables_json JSONB NOT NULL DEFAULT '[]'::jsonb,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    FOREIGN KEY (user_id, matter_id) REFERENCES matters(user_id, matter_id) ON DELETE CASCADE,
    UNIQUE (user_id, matter_id, name)
);

CREATE INDEX IF NOT EXISTS idx_document_templates_user_matter
    ON document_templates(user_id, matter_id);
CREATE INDEX IF NOT EXISTS idx_document_templates_user_name
    ON document_templates(user_id, name);
