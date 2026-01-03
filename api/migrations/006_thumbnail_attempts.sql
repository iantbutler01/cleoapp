-- Track thumbnail generation attempts to avoid infinite retry loops
ALTER TABLE captures ADD COLUMN thumbnail_attempts INT NOT NULL DEFAULT 0;

-- Update the index to exclude captures that have failed too many times (5+ attempts)
DROP INDEX IF EXISTS idx_captures_no_thumbnail;
CREATE INDEX idx_captures_needs_thumbnail ON captures (captured_at DESC)
WHERE thumbnail_path IS NULL AND thumbnail_attempts < 5;
