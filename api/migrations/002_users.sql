-- Users table with Twitter OAuth 2.0 credentials
CREATE TABLE users (
    id BIGSERIAL PRIMARY KEY,
    twitter_id TEXT UNIQUE NOT NULL,
    twitter_username TEXT NOT NULL,
    twitter_name TEXT,
    access_token TEXT NOT NULL,
    refresh_token TEXT,
    token_expires_at TIMESTAMPTZ NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    updated_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_users_twitter_id ON users (twitter_id);

-- OAuth state storage (temporary, for PKCE flow)
CREATE TABLE oauth_states (
    state TEXT PRIMARY KEY,
    code_verifier TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

-- Cleanup old oauth states (older than 10 minutes)
CREATE INDEX idx_oauth_states_created ON oauth_states (created_at);

-- Add foreign key constraint to captures
ALTER TABLE captures ADD CONSTRAINT fk_captures_user FOREIGN KEY (user_id) REFERENCES users(id);

-- Add foreign key constraint to agent_runs
ALTER TABLE agent_runs ADD CONSTRAINT fk_agent_runs_user FOREIGN KEY (user_id) REFERENCES users(id);

-- Add foreign key constraint to tweet_collateral
ALTER TABLE tweet_collateral ADD CONSTRAINT fk_tweet_collateral_user FOREIGN KEY (user_id) REFERENCES users(id);
