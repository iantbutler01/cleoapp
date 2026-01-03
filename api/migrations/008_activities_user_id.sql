-- Add user_id to activities table for proper data isolation
-- Previously activities only had interval_id which required joining through captures

-- Add user_id column (nullable initially for existing data)
ALTER TABLE activities ADD COLUMN user_id BIGINT;

-- Backfill user_id from captures via interval_id
-- This associates existing activities with the user who created captures in the same interval
UPDATE activities a
SET user_id = (
    SELECT DISTINCT c.user_id
    FROM captures c
    WHERE c.interval_id = a.interval_id
    LIMIT 1
);

-- Make user_id NOT NULL after backfill (activities without matching captures will be orphaned)
-- Note: If there are orphaned activities, this will fail - delete them first if needed
ALTER TABLE activities ALTER COLUMN user_id SET NOT NULL;

-- Add index for efficient user+timestamp queries (matches captures pattern)
CREATE INDEX idx_activities_user ON activities (user_id, timestamp DESC);
