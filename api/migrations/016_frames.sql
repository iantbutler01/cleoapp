ALTER TABLE captures ADD COLUMN frames_extracted BOOLEAN DEFAULT FALSE;
ALTER TABLE captures ADD COLUMN frame_count INTEGER;
ALTER TABLE captures ADD COLUMN frame_attempts INTEGER DEFAULT 0;

CREATE INDEX idx_captures_frames_pending
  ON captures (captured_at DESC)
  WHERE frames_extracted = FALSE AND frame_attempts < 5;
