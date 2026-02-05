-- Clean up previous moments tables (if they exist from abandoned approach)
DROP TABLE IF EXISTS moment_media CASCADE;
DROP TABLE IF EXISTS moment_copies CASCADE;
DROP TABLE IF EXISTS moments CASCADE;

-- Add copy/media variation columns to tweet_collateral
-- Primary selection stays in: text, video_clip, image_capture_ids
-- Alternatives go in: copy_options, media_options
ALTER TABLE tweet_collateral ADD COLUMN IF NOT EXISTS copy_options JSONB DEFAULT '[]';
ALTER TABLE tweet_collateral ADD COLUMN IF NOT EXISTS media_options JSONB DEFAULT '[]';

-- For threads: array of thread variations where each variation is an array of tweet texts
-- Example: [["tweet1 v1", "tweet2 v1"], ["tweet1 v2", "tweet2 v2"]]
ALTER TABLE tweet_threads ADD COLUMN IF NOT EXISTS copy_options JSONB DEFAULT '[]';
