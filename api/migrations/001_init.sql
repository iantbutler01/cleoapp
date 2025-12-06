CREATE EXTENSION IF NOT EXISTS timescaledb;

CREATE TABLE captures (
    id BIGSERIAL,
    interval_id BIGINT NOT NULL,
    user_id BIGINT NOT NULL,
    media_type TEXT NOT NULL,
    content_type TEXT NOT NULL,
    gcs_path TEXT NOT NULL,
    captured_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    PRIMARY KEY (id, captured_at)
);

SELECT create_hypertable('captures', 'captured_at');

CREATE TABLE activities (
    id BIGSERIAL,
    "timestamp" TIMESTAMPTZ NOT NULL,
    interval_id BIGINT NOT NULL,
    event_type TEXT NOT NULL,
    application TEXT,
    "window" TEXT,
    PRIMARY KEY (id, "timestamp")
);

SELECT create_hypertable('activities', 'timestamp');

CREATE INDEX idx_captures_interval ON captures (interval_id, captured_at DESC);
CREATE INDEX idx_captures_user ON captures (user_id, captured_at DESC);
CREATE INDEX idx_activities_interval ON activities (interval_id, timestamp DESC);
CREATE INDEX idx_activities_event_type ON activities (event_type, timestamp DESC);

-- Agent run tracking
CREATE TABLE agent_runs (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL,
    window_start TIMESTAMPTZ NOT NULL,
    window_end TIMESTAMPTZ NOT NULL,
    tweets_generated INT NOT NULL DEFAULT 0,
    completed_at TIMESTAMPTZ NOT NULL DEFAULT NOW()
);

CREATE INDEX idx_agent_runs_user ON agent_runs (user_id, completed_at DESC);

-- Tweet collateral output
CREATE TABLE tweet_collateral (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL,
    text TEXT NOT NULL,
    video_clip JSONB,
    image_capture_ids BIGINT[] DEFAULT '{}',
    rationale TEXT NOT NULL,
    created_at TIMESTAMPTZ NOT NULL DEFAULT NOW(),
    posted_at TIMESTAMPTZ,
    tweet_id TEXT
);

CREATE INDEX idx_tweet_collateral_user ON tweet_collateral (user_id, created_at DESC);
CREATE INDEX idx_tweet_collateral_unposted ON tweet_collateral (user_id) WHERE posted_at IS NULL;
