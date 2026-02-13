ALTER TABLE tweet_collateral
    ADD COLUMN IF NOT EXISTS publish_status TEXT NOT NULL DEFAULT 'pending',
    ADD COLUMN IF NOT EXISTS publish_attempts INT NOT NULL DEFAULT 0,
    ADD COLUMN IF NOT EXISTS publish_error TEXT,
    ADD COLUMN IF NOT EXISTS publish_error_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_tweet_collateral_publish_status
    ON tweet_collateral (user_id, publish_status)
    WHERE posted_at IS NULL;
