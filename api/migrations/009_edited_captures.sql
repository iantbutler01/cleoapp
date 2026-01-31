-- Add support for edited captures (derived from source captures)
-- source_capture_id: references the original capture this was derived from
-- edit_params: JSON containing the edit operation (crop, trim, etc.)

ALTER TABLE captures ADD COLUMN source_capture_id BIGINT;
ALTER TABLE captures ADD COLUMN edit_params JSONB;

-- Note: Not adding FK constraint because captures uses TimescaleDB hypertable
-- with composite primary key (id, captured_at), which complicates foreign keys.
-- The source_capture_id is validated at application level.

-- Index for finding all edits of a source capture
CREATE INDEX idx_captures_source_id ON captures (source_capture_id) WHERE source_capture_id IS NOT NULL;
