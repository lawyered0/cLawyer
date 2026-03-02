-- Bind conversations to a legal matter for hard matter isolation.
-- Nullable for legacy rows; matter binding is lazy on first legal use.

ALTER TABLE conversations
    ADD COLUMN IF NOT EXISTS matter_id TEXT;

CREATE INDEX IF NOT EXISTS idx_conversations_user_channel_matter_last_activity
    ON conversations(user_id, channel, matter_id, last_activity DESC);
