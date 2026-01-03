# Cleo Architecture Patterns & Code Review Summary

This document details the structural patterns established across the codebase after the comprehensive code review, along with identified inconsistencies that should be addressed over time.

---

## Backend Architecture (Rust/Axum)

### Layer Separation

The backend follows a three-layer architecture:

```
routes/          → HTTP handlers (request/response, auth, status codes)
domain/          → Business logic & database queries (pure data operations)
models.rs        → Shared data structures
services/        → External service integrations (Twitter API, sessions, cookies)
constants.rs     → Application-wide constants
```

### Pattern: Domain Layer (`api/src/domain/`)

**Purpose**: Pure database operations, no HTTP concerns.

**Consistent patterns**:
- Functions take `&PgPool` as first argument
- Return `Result<T, sqlx::Error>`
- No HTTP status codes or error mapping
- Named query functions: `list_*`, `get_*`, `count_*`, `create_*`, `delete_*`, `update_*`
- Status filtering via helper function

**Example** (`domain/tweets.rs:19-26`):
```rust
fn status_filter_clause(status_filter: Option<&str>) -> &'static str {
    match status_filter {
        Some("pending") => "AND posted_at IS NULL",
        Some("posted") => "AND posted_at IS NOT NULL",
        _ => "",
    }
}
```

**Example** (`domain/threads.rs:9-15`):
```rust
fn status_filter_clause(status_filter: Option<&str>) -> &'static str {
    match status_filter {
        Some("pending") => "AND status IN ('draft', 'partial_failed')",
        Some("posted") => "AND status = 'posted'",
        _ => "",
    }
}
```

### Pattern: Transactional Operations

**Multi-step mutations use explicit transactions** (`domain/threads.rs:171-211`):
```rust
pub async fn create_thread(...) -> Result<i64, sqlx::Error> {
    let mut tx = db.begin().await?;

    // Step 1: Insert thread
    let (thread_id,): (i64,) = sqlx::query_as(...)
        .fetch_one(&mut *tx).await?;

    // Step 2: Update related records
    for (position, tweet_id) in tweet_ids.iter().enumerate() {
        sqlx::query(...).execute(&mut *tx).await?;
    }

    tx.commit().await?;
    Ok(thread_id)
}
```

**Applied to**: `create_thread()`, `delete_thread()`

### Pattern: Batch Queries (N+1 Prevention)

**Problem solved**: Fetching threads with tweets was N+1 (1 query for threads + N queries for each thread's tweets).

**Solution** (`domain/threads.rs:136-198`):
```rust
pub async fn list_threads_with_tweets(...) -> Result<Vec<ThreadWithTweets>, sqlx::Error> {
    // Query 1: Get all threads
    let threads = list_threads(db, user_id, status_filter).await?;

    // Query 2: Batch fetch ALL tweets for ALL threads in one query
    let thread_ids: Vec<i64> = threads.iter().map(|t| t.id).collect();
    let all_tweets: Vec<ThreadTweetWithThreadId> = sqlx::query_as(
        "SELECT ... FROM tweet_collateral WHERE thread_id = ANY($1) ..."
    ).bind(&thread_ids).fetch_all(db).await?;

    // Group in memory
    let mut tweets_by_thread: HashMap<i64, Vec<ThreadTweet>> = HashMap::new();
    for tweet_row in all_tweets {
        tweets_by_thread.entry(tweet_row.thread_id).or_default().push(...);
    }

    // Assemble results
    threads.into_iter().map(|thread| {
        ThreadWithTweets { thread, tweets: tweets_by_thread.remove(&thread.id).unwrap_or_default() }
    }).collect()
}
```

**Also applied to media uploads** (`routes/content/twitter/media.rs:196-231`):
```rust
pub async fn fetch_captures_batch(...) -> Result<HashMap<i64, CaptureInfo>, String> {
    let rows: Vec<CaptureRow> = sqlx::query_as(
        "SELECT id, gcs_path, content_type FROM captures WHERE id = ANY($1) AND user_id = $2"
    ).bind(capture_ids).fetch_all(&state.db).await?;

    rows.into_iter().map(|row| (row.id, CaptureInfo { ... })).collect()
}
```

### Pattern: Pagination

**Standard pagination response structure**:
```rust
#[derive(Serialize)]
struct ListResponse {
    items: Vec<T>,      // or domain-specific name like `threads`, `tweets`
    total: i64,         // Total count from database
    has_more: bool,     // Whether more items exist beyond current page
}
```

**Pagination constants** (`constants.rs:12-16`):
```rust
pub const DEFAULT_PAGE_SIZE: i64 = 50;
pub const MAX_PAGE_SIZE: i64 = 100;
```

**Usage pattern** (`routes/content/twitter/threads.rs:98-99`):
```rust
let limit = query.limit.unwrap_or(DEFAULT_PAGE_SIZE).min(MAX_PAGE_SIZE);
let offset = query.offset.unwrap_or(0);
```

### Pattern: Typed Request Validation

**Strongly-typed request structs with serde** (`routes/content/twitter/threads.rs:494-506`):
```rust
/// Strongly-typed video clip for request validation
#[derive(Deserialize)]
struct VideoClipInput {
    source_capture_id: i64,
    start_timestamp: String,
    duration_secs: f64,
}

#[derive(Deserialize)]
struct UpdateCollateralRequest {
    image_capture_ids: Option<Vec<i64>>,
    video_clip: Option<VideoClipInput>,
}
```

**Conversion to JSON for DB storage** (`routes/content/twitter/threads.rs:545-552`):
```rust
let video_clip_json: Option<serde_json::Value> = payload.video_clip.as_ref().map(|vc| {
    serde_json::json!({
        "source_capture_id": vc.source_capture_id,
        "start_timestamp": vc.start_timestamp,
        "duration_secs": vc.duration_secs
    })
});
```

### Pattern: HTTP Status Codes

| Operation | Success | Failure |
|-----------|---------|---------|
| GET (found) | 200 OK | 404 NOT_FOUND |
| POST (create) | 201 CREATED | 400/409 |
| PUT (update) | 200 OK | 404/400/409 |
| DELETE | 204 NO_CONTENT | 404 |
| Auth failure | - | 401 UNAUTHORIZED |
| Server error | - | 500 INTERNAL_SERVER_ERROR |

---

## Frontend Architecture (TypeScript/Lit)

### Pattern: API Client with Zod Validation

**Runtime validation for all API responses** (`api.ts:7-126`):
```typescript
const VideoClipSchema = z.object({
  source_capture_id: z.number(),
  start_timestamp: z.string(),
  duration_secs: z.number(),
});

// Types inferred from Zod schemas
export type VideoClip = z.infer<typeof VideoClipSchema>;
```

**Validated fetch helper** (`api.ts:272-282`):
```typescript
private async fetchJson<T>(
  url: string,
  options: RequestInit,
  errorMessage: string,
  schema: z.ZodType<T>
): Promise<T> {
  const res = await this.fetchWithAuth(url, options);
  if (!res.ok) throw new Error(errorMessage);
  const data = await res.json();
  return schema.parse(data);  // Runtime validation
}
```

### Pattern: Client-Side Caching

**Signed URL caching** (`api.ts:179-181`):
```typescript
// Cache for capture URLs (signed URLs expire in 15 minutes, cache for 10)
private captureUrlCache = new Map<number, { data: { url: string; content_type: string }; expires: number }>();
private readonly CAPTURE_URL_CACHE_TTL = 10 * 60 * 1000; // 10 minutes
```

**Cache-first fetch pattern** (`api.ts:421-443`):
```typescript
async getCaptureUrl(captureId: number): Promise<{ url: string; content_type: string }> {
  // Check cache first
  const cached = this.captureUrlCache.get(captureId);
  if (cached && cached.expires > Date.now()) {
    return cached.data;
  }

  // Fetch fresh URL
  const data = await this.fetchJson(...);

  // Cache the result
  this.captureUrlCache.set(captureId, {
    data,
    expires: Date.now() + this.CAPTURE_URL_CACHE_TTL,
  });

  return data;
}
```

**Cache invalidation on logout** (`api.ts:312-313`):
```typescript
// Clear caches
this.captureUrlCache.clear();
```

### Pattern: Error State Management

**Component-level error states** (`login-page.ts:12`, `media-browser.ts:67`):
```typescript
@state() authError: string | null = null;
@state() loadError: string | null = null;
@state() saveError: string | null = null;
```

**Error UI with retry** (`tweet-content.ts:119-132`):
```typescript
if (this.mediaError) {
  return html`
    <div class="mt-3 p-4 rounded-lg bg-error/10 border border-error/20">
      <div class="flex items-center gap-2 text-error text-sm">
        <svg ...>...</svg>
        <span>${this.mediaError}</span>
      </div>
      <button class="btn btn-sm btn-ghost mt-2" @click=${this.loadMedia}>
        Retry
      </button>
    </div>
  `;
}
```

---

## Inconsistencies Identified

### 1. Inline SQL in Route Handlers

**Problem**: Some route handlers contain raw SQL instead of using domain functions.

**Locations**:
- `routes/content/twitter/threads.rs:174-198` (update_thread reorder logic)
- `routes/content/twitter/threads.rs:249-290` (add_tweet_to_thread)
- `routes/content/twitter/threads.rs:311-340` (remove_tweet_from_thread)
- `routes/content/twitter/threads.rs:516-537` (update_tweet_collateral validation)

**Should be**: Extracted to `domain/threads.rs` as:
- `reorder_thread_tweets(db, thread_id, user_id, tweet_ids)`
- `add_tweet_to_thread(db, thread_id, user_id, tweet_id, position)`
- `remove_tweet_from_thread(db, thread_id, user_id, tweet_id)`
- `validate_captures_ownership(db, user_id, capture_ids)`

### 2. Duplicate Status Filter Helpers

**Problem**: Two nearly identical `status_filter_clause` functions exist.

**Locations**:
- `domain/tweets.rs:19-26`
- `domain/threads.rs:9-15`

**Difference**: Threads use `status IN ('draft', 'partial_failed')` for pending, tweets use `posted_at IS NULL`.

**Recommendation**: Keep separate since they operate on different columns, but document the semantic difference clearly.

### 3. VideoClip Struct Duplication

**Problem**: VideoClip is defined in multiple places:

| Location | Name | Purpose |
|----------|------|---------|
| `models.rs:7-13` | `VideoClip` | Shared model with `from_json`/`to_json` helpers |
| `routes/.../threads.rs:494-500` | `VideoClipInput` | Request deserialization |
| `api.ts:7-11` | `VideoClipSchema` | Frontend Zod schema |

**Current state**: Acceptable - input validation structs are intentionally separate from domain models. The `VideoClip` in models.rs provides conversion helpers that aren't currently used but establish the pattern for future strongly-typed DB reads.

### 4. Mixed Error Handling Styles

**Problem**: Route handlers use different error patterns:

**Style A - Map to StatusCode immediately**:
```rust
.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?
```

**Style B - Log then map**:
```rust
.map_err(|e| {
    eprintln!("Error: {}", e);
    StatusCode::INTERNAL_SERVER_ERROR
})?
```

**Recommendation**: Standardize on Style B for debugging visibility, or implement proper error types.

### 5. Pagination in Content Endpoint

**Problem**: `routes/content/mod.rs:91-122` fetches all records then paginates in memory.

```rust
// Get standalone tweets (ALL of them)
let standalone = tweets::list_standalone_tweets(&state.db, user_id, status_filter).await?;

// Paginate in memory
let paginated: Vec<ContentItem> = items
    .into_iter()
    .skip(query.offset as usize)
    .take(query.limit as usize)
    .collect();
```

**Impact**: Acceptable for small datasets. For scale, would need a UNION query with proper LIMIT/OFFSET at DB level.

### 6. Inconsistent Default Page Sizes

**Problem**: `routes/content/mod.rs:53-55` uses a different default:
```rust
fn default_limit() -> i64 {
    500  // vs DEFAULT_PAGE_SIZE = 50 everywhere else
}
```

**Recommendation**: Use `DEFAULT_PAGE_SIZE` constant consistently.

### 7. Dead Code Warnings Suppressed

**Problem**: Several `#[allow(dead_code)]` annotations exist:
- `models.rs:8,15` - VideoClip struct and impl
- `models.rs:29` - CaptureRecord
- `domain/tweets.rs:29,114` - Some query functions

**Status**: Acceptable for now - these provide infrastructure for future features.

---

## Summary of Changes Made

### Phase 1: Backend Data Integrity
- ✅ Added transactions to `create_thread()` and `delete_thread()`
- ✅ Fixed N+1 in `list_threads_with_tweets()` with batch query
- ✅ Batched media queries in `upload_tweet_media()`

### Phase 2: Frontend Robustness
- ✅ Added Zod runtime validation to all API responses
- ✅ Added error states to components (login, media-browser, tweet-content)
- ✅ Added retry buttons for failed operations

### Phase 3: Pagination & Status Codes
- ✅ Added pagination constants to `constants.rs`
- ✅ Added pagination to tweets and threads list endpoints
- ✅ Fixed DELETE to return 204 NO_CONTENT

### Phase 4: Code Quality
- ✅ Extracted status filter helpers in domain layer
- ✅ Added `VideoClipInput` typed struct for request validation
- ✅ Added capture URL caching in frontend API client
