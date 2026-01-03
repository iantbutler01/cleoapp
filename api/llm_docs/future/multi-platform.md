# Future: Multi-Platform Content Publishing

## Context
When Cleo expands beyond Twitter to Bluesky, LinkedIn, and blogs.

## Key Insight
The unit of work becomes a "content piece" that gets adapted per platform, not individual tweets.

## Proposed Data Model

```sql
-- Replaces tweet_threads
content_items (
    id BIGSERIAL PRIMARY KEY,
    user_id BIGINT NOT NULL,
    title TEXT,
    status TEXT DEFAULT 'draft',
    created_at TIMESTAMPTZ,
    -- Which platforms to publish to
    platform_targets TEXT[] DEFAULT '{twitter}'
)

-- Replaces tweet_collateral
content_segments (
    id BIGSERIAL PRIMARY KEY,
    content_id BIGINT NOT NULL,  -- Always belongs to a content item
    position INT NOT NULL,
    text TEXT NOT NULL,
    media JSONB,
    rationale TEXT,
    -- Platform-specific overrides (e.g., shorter text for Twitter)
    platform_overrides JSONB DEFAULT '{}'
)

-- Track publishing status per platform
content_publications (
    id BIGSERIAL PRIMARY KEY,
    content_id BIGINT NOT NULL,
    platform TEXT NOT NULL,  -- 'twitter', 'bluesky', 'linkedin', 'blog'
    status TEXT DEFAULT 'pending',
    published_at TIMESTAMPTZ,
    platform_ids JSONB  -- e.g., {"tweets": ["123", "124"]} for Twitter thread
)
```

## Platform Adaptation Logic

| Platform  | Segments Behavior |
|-----------|-------------------|
| Twitter   | Each segment = tweet in reply chain |
| Bluesky   | Each segment = post in reply chain |
| LinkedIn  | Concatenate all segments into single long-form post |
| Blog      | Concatenate segments as markdown sections |

## Agent Tool

Single tool replaces WriteTweet + WriteThread:

```rust
WriteContent {
    title: Option<String>,
    segments: Vec<ContentSegment>,  // Always at least 1
    rationale: String,
}
```

## Migration Path
1. Rename `tweet_threads` → `content_items`
2. Rename `tweet_collateral` → `content_segments`
3. Make `content_id` NOT NULL (backfill standalone tweets as content items with 1 segment)
4. Add `platform_targets` and `content_publications`

## When to Implement
After Twitter posting is solid and there's actual demand for Bluesky/LinkedIn.
