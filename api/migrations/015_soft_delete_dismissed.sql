-- Soft delete for dismissed tweets: preserve data for accept/dismiss analytics and training
ALTER TABLE tweet_collateral
    ADD COLUMN IF NOT EXISTS dismissed_at TIMESTAMPTZ;

CREATE INDEX IF NOT EXISTS idx_tweet_collateral_dismissed
    ON tweet_collateral (user_id, dismissed_at)
    WHERE dismissed_at IS NOT NULL;
