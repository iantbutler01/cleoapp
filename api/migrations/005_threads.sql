-- Thread container for grouping related tweets
CREATE TABLE tweet_threads (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL REFERENCES users(id),
    title TEXT,
    status TEXT NOT NULL DEFAULT 'draft',  -- draft, posting, posted, partial_failed
    created_at TIMESTAMPTZ DEFAULT NOW(),
    posted_at TIMESTAMPTZ,
    first_tweet_id TEXT  -- Twitter ID of first posted tweet (for linking)
);

CREATE INDEX idx_tweet_threads_user ON tweet_threads (user_id, created_at DESC);
CREATE INDEX idx_tweet_threads_status ON tweet_threads (user_id, status);

-- Add thread reference to existing tweets
ALTER TABLE tweet_collateral
    ADD COLUMN thread_id BIGINT REFERENCES tweet_threads(id) ON DELETE SET NULL,
    ADD COLUMN thread_position INT,
    ADD COLUMN reply_to_tweet_id TEXT;  -- Twitter ID this replied to (after posting)

CREATE INDEX idx_tweet_collateral_thread ON tweet_collateral (thread_id, thread_position);
