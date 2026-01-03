-- Add thumbnail support to captures
ALTER TABLE captures ADD COLUMN thumbnail_path TEXT;

-- Index for finding captures without thumbnails (for batch processing)
CREATE INDEX idx_captures_no_thumbnail ON captures (captured_at DESC)
WHERE thumbnail_path IS NULL;
