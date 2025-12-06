-- Add API token column for daemon authentication
ALTER TABLE users ADD COLUMN api_token TEXT UNIQUE;

CREATE INDEX idx_users_api_token ON users (api_token) WHERE api_token IS NOT NULL;
