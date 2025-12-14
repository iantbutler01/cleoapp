-- Seed data for local development
-- Assumes you've already logged in via Twitter (user_id = 1)
--
-- Usage:
--   1. Copy fixture media to your LOCAL_STORAGE_PATH:
--      cp -r fixtures/media/* $LOCAL_STORAGE_PATH/
--   2. Run this SQL:
--      psql -d cleo -f fixtures/seed.sql

-- Create sample captures (screenshots)
-- These reference files in LOCAL_STORAGE_PATH: image/user_1/2025-12-12/
INSERT INTO captures (id, interval_id, user_id, media_type, content_type, gcs_path, captured_at)
VALUES
    (1, 1000, 1, 'image', 'image/png', 'image/user_1/2025-12-12/capture_001.png', NOW() - INTERVAL '2 hours'),
    (2, 1000, 1, 'image', 'image/png', 'image/user_1/2025-12-12/capture_002.png', NOW() - INTERVAL '1 hour 55 minutes'),
    (3, 1001, 1, 'image', 'image/png', 'image/user_1/2025-12-12/capture_003.png', NOW() - INTERVAL '1 hour 30 minutes'),
    (4, 1001, 1, 'image', 'image/png', 'image/user_1/2025-12-12/capture_004.png', NOW() - INTERVAL '1 hour'),
    (5, 1002, 1, 'image', 'image/png', 'image/user_1/2025-12-12/capture_005.png', NOW() - INTERVAL '30 minutes')
ON CONFLICT DO NOTHING;

SELECT setval('captures_id_seq', (SELECT COALESCE(MAX(id), 1) FROM captures));

-- Create sample tweet suggestions with attached images
INSERT INTO tweet_collateral (id, user_id, text, image_capture_ids, rationale, created_at)
VALUES
    (1, 1,
     'Just shipped a new feature! The local storage integration is working beautifully now. No more cloud dependencies for dev mode.',
     ARRAY[1, 2],
     'Developer showing progress on a technical feature - captures show code editor with storage implementation',
     NOW() - INTERVAL '1 hour'),

    (2, 1,
     'Debugging session complete. Found the race condition that was causing intermittent failures. Sometimes the simplest bugs hide in the most obvious places.',
     ARRAY[3],
     'Capture shows terminal with successful test output after debugging session',
     NOW() - INTERVAL '45 minutes'),

    (3, 1,
     'Building in public: Added seed data support so I can actually see what the dashboard looks like with real content. Small wins matter.',
     ARRAY[4, 5],
     'Screenshots show the web dashboard with tweet cards displaying - meta moment of building the tool',
     NOW() - INTERVAL '15 minutes')
ON CONFLICT DO NOTHING;

SELECT setval('tweet_collateral_id_seq', (SELECT COALESCE(MAX(id), 1) FROM tweet_collateral));

-- Summary
SELECT 'Seed data loaded!' AS status;
SELECT COUNT(*) AS captures FROM captures WHERE user_id = 1;
SELECT COUNT(*) AS pending_tweets FROM tweet_collateral WHERE user_id = 1 AND posted_at IS NULL;
