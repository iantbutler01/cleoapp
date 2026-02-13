ALTER TABLE agent_runs
    ADD COLUMN IF NOT EXISTS status TEXT NOT NULL DEFAULT 'completed',
    ADD COLUMN IF NOT EXISTS started_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS error_message TEXT,
    ADD COLUMN IF NOT EXISTS attempts INT NOT NULL DEFAULT 0;

CREATE UNIQUE INDEX IF NOT EXISTS idx_agent_runs_user_running
    ON agent_runs (user_id)
    WHERE status = 'running';

CREATE INDEX IF NOT EXISTS idx_agent_runs_user_status
    ON agent_runs (user_id, status, completed_at);
