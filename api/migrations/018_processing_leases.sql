ALTER TABLE captures
    ADD COLUMN IF NOT EXISTS frames_processing_started_at TIMESTAMPTZ,
    ADD COLUMN IF NOT EXISTS thumbnail_processing BOOLEAN NOT NULL DEFAULT FALSE,
    ADD COLUMN IF NOT EXISTS thumbnail_processing_started_at TIMESTAMPTZ;

UPDATE captures
SET frames_processing_started_at = NOW() - INTERVAL '1 day'
WHERE frames_processing = TRUE
  AND frames_processing_started_at IS NULL;

UPDATE captures
SET thumbnail_processing = FALSE,
    thumbnail_processing_started_at = NULL
WHERE thumbnail_path IS NOT NULL;

UPDATE captures
SET thumbnail_processing_started_at = NOW() - INTERVAL '1 day'
WHERE thumbnail_processing = TRUE
  AND thumbnail_processing_started_at IS NULL;

CREATE INDEX IF NOT EXISTS idx_captures_frames_claim
    ON captures (frames_processing, frames_processing_started_at, captured_at ASC)
    WHERE frames_extracted = FALSE AND frame_attempts < 5;

CREATE INDEX IF NOT EXISTS idx_captures_thumbnails_claim
    ON captures (thumbnail_processing, thumbnail_processing_started_at, captured_at ASC)
    WHERE thumbnail_path IS NULL AND thumbnail_attempts < 5;
