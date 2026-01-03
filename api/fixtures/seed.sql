-- Comprehensive seed data for local development
-- Creates captures and tweets across multiple days/hours for user_id = 1
--
-- Usage:
--   1. Copy fixture media to your LOCAL_STORAGE_PATH:
--      cp -r fixtures/media/* $LOCAL_STORAGE_PATH/
--   2. Run this SQL:
--      psql -d cleo -f fixtures/seed.sql
--
-- Available fixture files (reused across dates):
--   image/user_1/2025-12-12/capture_001.png through capture_005.png
--   video/user_1/2025-12-12/recording_001.mov

-- Clear previous seed data (keeps users intact)
TRUNCATE tweet_threads RESTART IDENTITY CASCADE;
TRUNCATE tweet_collateral RESTART IDENTITY CASCADE;
TRUNCATE captures RESTART IDENTITY CASCADE;
TRUNCATE activities RESTART IDENTITY CASCADE;
TRUNCATE agent_runs RESTART IDENTITY CASCADE;

-- Helper: Generate captures across multiple days
-- We'll create ~100 captures over the past 7 days, varying times of day
DO $$
DECLARE
    capture_id INTEGER := 1;
    interval_id INTEGER := 1000;
    days_ago INTEGER;
    hour_offset INTEGER;
    minute_offset INTEGER;
    capture_num INTEGER;
    image_file TEXT;
    capture_time TIMESTAMPTZ;
BEGIN
    -- Loop through last 7 days (0 = today, 6 = 6 days ago)
    FOR days_ago IN 0..6 LOOP
        -- Each day has ~10-20 captures at various hours
        -- Morning work session (9-12)
        FOR hour_offset IN 9..11 LOOP
            FOR capture_num IN 1..3 LOOP
                minute_offset := (capture_num - 1) * 15 + floor(random() * 10)::int;
                image_file := 'image/user_1/2025-12-12/capture_00' || ((capture_id % 5) + 1) || '.png';
                capture_time := NOW() - (days_ago || ' days')::interval - (hour_offset || ' hours')::interval - (minute_offset || ' minutes')::interval;

                INSERT INTO captures (id, interval_id, user_id, media_type, content_type, gcs_path, captured_at)
                VALUES (capture_id, interval_id, 1, 'image', 'image/png', image_file, capture_time);

                capture_id := capture_id + 1;
            END LOOP;
            interval_id := interval_id + 1;
        END LOOP;

        -- Afternoon session (14-17)
        FOR hour_offset IN 14..16 LOOP
            FOR capture_num IN 1..2 LOOP
                minute_offset := (capture_num - 1) * 20 + floor(random() * 15)::int;
                image_file := 'image/user_1/2025-12-12/capture_00' || ((capture_id % 5) + 1) || '.png';
                capture_time := NOW() - (days_ago || ' days')::interval - (hour_offset || ' hours')::interval - (minute_offset || ' minutes')::interval;

                INSERT INTO captures (id, interval_id, user_id, media_type, content_type, gcs_path, captured_at)
                VALUES (capture_id, interval_id, 1, 'image', 'image/png', image_file, capture_time);

                capture_id := capture_id + 1;
            END LOOP;
            interval_id := interval_id + 1;
        END LOOP;

        -- Evening session (20-23) - some days only
        IF days_ago % 2 = 0 THEN
            FOR hour_offset IN 20..22 LOOP
                minute_offset := floor(random() * 45)::int;
                image_file := 'image/user_1/2025-12-12/capture_00' || ((capture_id % 5) + 1) || '.png';
                capture_time := NOW() - (days_ago || ' days')::interval - (hour_offset || ' hours')::interval - (minute_offset || ' minutes')::interval;

                INSERT INTO captures (id, interval_id, user_id, media_type, content_type, gcs_path, captured_at)
                VALUES (capture_id, interval_id, 1, 'image', 'image/png', image_file, capture_time);

                capture_id := capture_id + 1;
                interval_id := interval_id + 1;
            END LOOP;
        END IF;

        -- One video per day
        capture_time := NOW() - (days_ago || ' days')::interval - '12 hours'::interval;
        INSERT INTO captures (id, interval_id, user_id, media_type, content_type, gcs_path, captured_at)
        VALUES (capture_id, interval_id, 1, 'video', 'video/mp4', 'video/user_1/2025-12-12/recording_001.mp4', capture_time);
        capture_id := capture_id + 1;
        interval_id := interval_id + 1;
    END LOOP;

    -- Update sequence
    PERFORM setval('captures_id_seq', capture_id);
END $$;

-- Create sample activities (for context)
INSERT INTO activities (user_id, interval_id, timestamp, event_type, application, "window")
SELECT
    1,  -- user_id
    1000 + (i % 50),
    NOW() - ((i * 15) || ' minutes')::interval,
    CASE (i % 4)
        WHEN 0 THEN 'app_switch'
        WHEN 1 THEN 'typing'
        WHEN 2 THEN 'mouse_click'
        ELSE 'scroll'
    END,
    CASE (i % 5)
        WHEN 0 THEN 'Code'
        WHEN 1 THEN 'Terminal'
        WHEN 2 THEN 'Safari'
        WHEN 3 THEN 'Slack'
        ELSE 'Finder'
    END,
    CASE (i % 5)
        WHEN 0 THEN 'main.rs - api'
        WHEN 1 THEN 'zsh'
        WHEN 2 THEN 'GitHub - Pull Request'
        WHEN 3 THEN '#engineering'
        ELSE 'Documents'
    END
FROM generate_series(1, 200) AS i;

-- ============================================
-- THREADS: Sample thread data
-- ============================================

-- Thread 1: Today - A debugging journey (3 tweets)
INSERT INTO tweet_threads (id, user_id, title, status, created_at)
VALUES (1, 1, 'The Great Memory Leak Hunt', 'draft', NOW() - INTERVAL '1 hour');

INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, thread_id, thread_position)
VALUES
    (1,
     'Alright, time to hunt down this memory leak that''s been haunting our production server for a week. Thread incoming.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 1)),
     'Opening hook for debugging thread - shows htop with growing memory usage',
     NOW() - INTERVAL '1 hour',
     1, 0),
    (1,
     'First suspect: the event listener cleanup. Nope, those are fine. Second suspect: caching layer. Getting warmer...',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 1 OFFSET 1)),
     'Middle of investigation - code showing cache implementation',
     NOW() - INTERVAL '55 minutes',
     1, 1),
    (1,
     'Found it! We were storing closures in a Map that never got cleared. 3 lines of code, 1 week of pain. Always clear your caches, folks.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 2 OFFSET 2)),
     'Resolution with before/after memory graphs',
     NOW() - INTERVAL '50 minutes',
     1, 2);

-- Thread 2: Yesterday - Building a feature (4 tweets, already posted)
INSERT INTO tweet_threads (id, user_id, title, status, created_at, posted_at, first_tweet_id)
VALUES (2, 1, 'Adding Dark Mode', 'posted', NOW() - INTERVAL '1 day 2 hours', NOW() - INTERVAL '1 day 1 hour', '9876543210');

INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, thread_id, thread_position, posted_at, tweet_id, reply_to_tweet_id)
VALUES
    (1,
     'Finally adding dark mode to the app. Here''s how I''m approaching it with CSS custom properties.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 1)),
     'Thread about implementing dark mode - shows CSS variables',
     NOW() - INTERVAL '1 day 2 hours',
     2, 0,
     NOW() - INTERVAL '1 day 1 hour',
     '9876543210',
     NULL),
    (1,
     'Step 1: Define your color tokens. I''m using semantic names like --color-bg-primary instead of --color-gray-100. Makes theming so much easier.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 1 OFFSET 1)),
     'Code showing CSS custom property definitions',
     NOW() - INTERVAL '1 day 1 hour 55 minutes',
     2, 1,
     NOW() - INTERVAL '1 day 58 minutes',
     '9876543211',
     '9876543210'),
    (1,
     'Step 2: Add a data-theme attribute to <html> and swap values. No JavaScript frameworks needed, just good old CSS.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'HTML showing theme attribute implementation',
     NOW() - INTERVAL '1 day 1 hour 50 minutes',
     2, 2,
     NOW() - INTERVAL '1 day 55 minutes',
     '9876543212',
     '9876543211'),
    (1,
     'And here''s the final result! Smooth transitions, respects system preference, and persists to localStorage. Dark mode done right.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 2 OFFSET 3)),
     'Before/after showing light and dark mode',
     NOW() - INTERVAL '1 day 1 hour 45 minutes',
     2, 3,
     NOW() - INTERVAL '1 day 52 minutes',
     '9876543213',
     '9876543212');

-- Thread 3: 3 days ago - Learning something new (5 tweets, draft)
INSERT INTO tweet_threads (id, user_id, title, status, created_at)
VALUES (3, 1, 'Rust Lifetimes Finally Click', 'draft', NOW() - INTERVAL '3 days 3 hours');

INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, thread_id, thread_position)
VALUES
    (1,
     'After 6 months of fighting the borrow checker, Rust lifetimes finally clicked today. Let me explain what made it make sense.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '3 days' ORDER BY captured_at DESC LIMIT 1)),
     'Opening about Rust learning journey',
     NOW() - INTERVAL '3 days 3 hours',
     3, 0),
    (1,
     'The key insight: lifetimes aren''t about how long data lives. They''re about proving to the compiler that references are valid.',
     NULL,
     ARRAY[]::BIGINT[],
     'Core concept explanation',
     NOW() - INTERVAL '3 days 2 hours 55 minutes',
     3, 1),
    (1,
     'Think of ''a as a label, not a timer. You''re saying "this reference and that reference share the same validity scope."',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '3 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 1)),
     'Analogy to help understanding',
     NOW() - INTERVAL '3 days 2 hours 50 minutes',
     3, 2),
    (1,
     'The compiler is just asking: "prove to me this reference won''t dangle." Lifetimes are your proof.',
     NULL,
     ARRAY[]::BIGINT[],
     'Compiler perspective',
     NOW() - INTERVAL '3 days 2 hours 45 minutes',
     3, 3),
    (1,
     'Resources that helped: The Rustonomicon, Jon Gjengset''s videos, and honestly just writing a lot of broken code. Keep at it!',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '3 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'Closing with resources',
     NOW() - INTERVAL '3 days 2 hours 40 minutes',
     3, 4);

-- Update sequence for threads
SELECT setval('tweet_threads_id_seq', 4);

-- ============================================
-- STANDALONE TWEETS (no thread)
-- ============================================

-- Today's tweets (pending)
INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at)
VALUES
    (1,
     'Just shipped a new feature! The local storage integration is working beautifully now. No more cloud dependencies for dev mode.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 2)),
     'Developer showing progress on a technical feature - captures show code editor with storage implementation',
     NOW() - INTERVAL '2 hours'),

    (1,
     'Debugging session complete. Found the race condition that was causing intermittent failures. Sometimes the simplest bugs hide in the most obvious places.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'Capture shows terminal with successful test output after debugging session',
     NOW() - INTERVAL '1 hour 30 minutes'),

    (1,
     'Building in public: Added seed data support so I can actually see what the dashboard looks like with real content. Small wins matter.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' ORDER BY captured_at DESC LIMIT 2 OFFSET 3)),
     'Screenshots show the web dashboard with tweet cards displaying - meta moment of building the tool',
     NOW() - INTERVAL '45 minutes'),

    (1,
     'Watch me accidentally break prod in real-time. Just kidding, this is local dev.',
     (SELECT json_build_object('source_capture_id', id, 'start_timestamp', '00:00:00', 'duration_secs', 5)::jsonb
      FROM captures WHERE user_id = 1 AND media_type = 'video' ORDER BY captured_at DESC LIMIT 1),
     ARRAY[]::BIGINT[],
     'Screen recording showing rapid terminal commands and code changes',
     NOW() - INTERVAL '15 minutes');

-- Yesterday's tweets (pending)
INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at)
VALUES
    (1,
     'Late night coding session paying off. The architecture is finally clicking into place.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 1)),
     'Screenshot of IDE with clean architecture diagram visible',
     NOW() - INTERVAL '1 day 3 hours'),

    (1,
     'Hot take: The best code is the code you delete. Just removed 200 lines and the feature works better now.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 2 OFFSET 1)),
     'Git diff showing significant code deletion with green tests',
     NOW() - INTERVAL '1 day 1 hour'),

    (1,
     'TIL: Sometimes the bug is in the test, not the code. Spent 2 hours debugging the wrong thing.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '1 day' ORDER BY captured_at DESC LIMIT 1 OFFSET 3)),
     'Terminal showing test output with highlighted failing assertion',
     NOW() - INTERVAL '1 day');

-- 2 days ago (pending)
INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at)
VALUES
    (1,
     'Refactoring day. The codebase thanks me, my brain does not.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '2 days' ORDER BY captured_at DESC LIMIT 3)),
     'Multiple screenshots showing before/after of refactored modules',
     NOW() - INTERVAL '2 days 4 hours'),

    (1,
     'Finally got the CI pipeline green after 47 attempts. We celebrate the small victories here.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '2 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 3)),
     'GitHub Actions showing green checkmarks across all jobs',
     NOW() - INTERVAL '2 days 2 hours');

-- 3-4 days ago (some already posted)
INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, posted_at, tweet_id)
VALUES
    (1,
     'Documentation day. Future me will thank present me. Probably.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '3 days' ORDER BY captured_at DESC LIMIT 2)),
     'Screenshots of well-documented code with JSDoc comments',
     NOW() - INTERVAL '3 days 5 hours',
     NOW() - INTERVAL '3 days 4 hours',
     '1234567890'),

    (1,
     'The moment when your side project starts feeling like a real product. Impostor syndrome, meet delusion of grandeur.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '3 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'Dashboard screenshot showing polished UI',
     NOW() - INTERVAL '3 days 2 hours',
     NOW() - INTERVAL '3 days 1 hour',
     '1234567891'),

    (1,
     'Spent the morning wrestling with Postgres. Postgres won, but I learned some new tricks.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '4 days' ORDER BY captured_at DESC LIMIT 2)),
     'Query plan visualization showing optimization',
     NOW() - INTERVAL '4 days 6 hours',
     NOW() - INTERVAL '4 days 5 hours',
     '1234567892');

-- More pending tweets (older)
INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at)
VALUES
    (1,
     'Pro tip: Coffee before coding, not after. The order matters.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '4 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'Screenshot showing early morning commit timestamps',
     NOW() - INTERVAL '4 days 3 hours'),

    (1,
     'The satisfaction of a clean git log is unmatched. Squash your commits, people.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '5 days' ORDER BY captured_at DESC LIMIT 2)),
     'Git log showing well-structured commit history',
     NOW() - INTERVAL '5 days 4 hours'),

    (1,
     'TypeScript saved me from myself today. Three hours of debugging prevented by a single type annotation.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '5 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'IDE showing TypeScript error that caught a bug',
     NOW() - INTERVAL '5 days 2 hours'),

    (1,
     'Pair programming with an AI is like having a colleague who never needs coffee breaks but occasionally hallucinates.',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '6 days' ORDER BY captured_at DESC LIMIT 2)),
     'Screenshot showing AI code assistant in action',
     NOW() - INTERVAL '6 days 5 hours'),

    (1,
     'Weekend project is now a weeknight project is now a month-long obsession. Send help (or PRs).',
     NULL,
     (SELECT ARRAY(SELECT id FROM captures WHERE user_id = 1 AND media_type = 'image' AND captured_at < NOW() - INTERVAL '6 days' ORDER BY captured_at DESC LIMIT 1 OFFSET 2)),
     'GitHub insights showing consistent commit activity',
     NOW() - INTERVAL '6 days 3 hours');

-- Create some agent runs to show history
INSERT INTO agent_runs (user_id, window_start, window_end, tweets_generated, completed_at)
VALUES
    (1, NOW() - INTERVAL '2 hours 30 minutes', NOW() - INTERVAL '2 hours', 2, NOW() - INTERVAL '1 hour 55 minutes'),
    (1, NOW() - INTERVAL '1 day 4 hours', NOW() - INTERVAL '1 day 3 hours', 3, NOW() - INTERVAL '1 day 2 hours 50 minutes'),
    (1, NOW() - INTERVAL '2 days 5 hours', NOW() - INTERVAL '2 days 4 hours', 2, NOW() - INTERVAL '2 days 3 hours 55 minutes'),
    (1, NOW() - INTERVAL '3 days 6 hours', NOW() - INTERVAL '3 days 5 hours', 2, NOW() - INTERVAL '3 days 4 hours 55 minutes'),
    (1, NOW() - INTERVAL '4 days 7 hours', NOW() - INTERVAL '4 days 6 hours', 2, NOW() - INTERVAL '4 days 5 hours 55 minutes'),
    (1, NOW() - INTERVAL '5 days 5 hours', NOW() - INTERVAL '5 days 4 hours', 2, NOW() - INTERVAL '5 days 3 hours 55 minutes'),
    (1, NOW() - INTERVAL '6 days 6 hours', NOW() - INTERVAL '6 days 5 hours', 2, NOW() - INTERVAL '6 days 4 hours 55 minutes');

-- Summary
SELECT '=== Seed Data Summary ===' AS info;
SELECT COUNT(*) AS total_captures FROM captures WHERE user_id = 1;
SELECT COUNT(*) AS total_threads FROM tweet_threads WHERE user_id = 1;
SELECT COUNT(*) AS pending_threads FROM tweet_threads WHERE user_id = 1 AND status = 'draft';
SELECT COUNT(*) AS posted_threads FROM tweet_threads WHERE user_id = 1 AND status = 'posted';
SELECT COUNT(*) AS standalone_pending_tweets FROM tweet_collateral WHERE user_id = 1 AND posted_at IS NULL AND thread_id IS NULL;
SELECT COUNT(*) AS standalone_posted_tweets FROM tweet_collateral WHERE user_id = 1 AND posted_at IS NOT NULL AND thread_id IS NULL;
SELECT COUNT(*) AS tweets_in_threads FROM tweet_collateral WHERE user_id = 1 AND thread_id IS NOT NULL;
SELECT COUNT(*) AS agent_runs FROM agent_runs WHERE user_id = 1;
SELECT MIN(captured_at)::date AS earliest_capture, MAX(captured_at)::date AS latest_capture FROM captures WHERE user_id = 1;
