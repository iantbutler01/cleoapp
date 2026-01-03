-- Refresh tokens table for session management
CREATE TABLE refresh_tokens (
    id TEXT PRIMARY KEY,                    -- random 32-byte hex token
    user_id BIGINT NOT NULL REFERENCES users(id) ON DELETE CASCADE,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    expires_at TIMESTAMPTZ NOT NULL,        -- 30 days from creation
    last_used_at TIMESTAMPTZ
);

CREATE INDEX idx_refresh_tokens_user ON refresh_tokens (user_id);
CREATE INDEX idx_refresh_tokens_expires ON refresh_tokens (expires_at);
