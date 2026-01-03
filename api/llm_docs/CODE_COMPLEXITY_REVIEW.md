# Code Complexity & Verbosity Review

**Date:** December 28, 2025
**Scope:** Rust backend (`api/src/`) and TypeScript/Lit frontend (`web/src/`)

This document identifies specific instances of unnecessary complexity and verbose code patterns, with concrete file locations, code snippets, and suggested simplifications.

---

## Part 1: Rust Backend (api/src/)

### Finding 1: Repetitive SQL Match Arms in Tweet Queries

**File:** `api/src/domain/tweets.rs`
**Lines:** 108-180 (`list_pending_tweets_paginated`), 182-259 (`list_standalone_tweets_paginated`)

**The Problem:**

Three nearly identical SQL query branches repeated in multiple functions. The only difference is the WHERE clause condition:

```rust
let rows: Vec<(i64, String, Option<serde_json::Value>, Vec<i64>, String, DateTime<Utc>)> =
    match filter {
        StatusFilter::Pending => {
            sqlx::query_as(
                r#"
                SELECT id, text, video_clip, image_capture_ids, rationale, created_at
                FROM tweet_collateral
                WHERE user_id = $1 AND thread_id IS NULL AND posted_at IS NULL
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#
            )
            .bind(user_id)
            .bind(limit)
            .bind(offset)
            .fetch_all(db)
            .await?
        }
        StatusFilter::Posted => {
            sqlx::query_as(
                r#"
                SELECT id, text, video_clip, image_capture_ids, rationale, created_at
                FROM tweet_collateral
                WHERE user_id = $1 AND thread_id IS NULL AND posted_at IS NOT NULL
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#
            )
            // ... same .bind() chain repeated
        }
        StatusFilter::All => {
            sqlx::query_as(
                r#"
                SELECT id, text, video_clip, image_capture_ids, rationale, created_at
                FROM tweet_collateral
                WHERE user_id = $1 AND thread_id IS NULL
                ORDER BY created_at DESC
                LIMIT $2 OFFSET $3
                "#
            )
            // ... same .bind() chain repeated
        }
    };
```

**Why It's Bad:**
- 90+ lines of duplicated logic
- Any query fix must be applied in 3 places
- Same pattern repeated in `count_standalone_tweets()` and similar functions

**Suggested Fix:**

```rust
fn status_where_clause(filter: StatusFilter) -> &'static str {
    match filter {
        StatusFilter::Pending => "AND posted_at IS NULL",
        StatusFilter::Posted => "AND posted_at IS NOT NULL",
        StatusFilter::All => "",
    }
}

// Then use format!() or sqlx::query! with runtime string
```

**Impact:** ~90 lines of duplication removed

---

### Finding 2: Triple-Match Pattern in Threads Domain

**File:** `api/src/domain/threads.rs`
**Lines:** 34-62, 76-115, 143-192

**The Problem:**

Identical three-way match pattern (Pending, Posted, All) repeated across three functions:

```rust
// count_threads - lines 34-62
let (count,): (i64,) = match filter {
    ThreadStatusFilter::Pending => {
        sqlx::query_as(
            "SELECT COUNT(*) FROM tweet_threads WHERE user_id = $1 AND status IN ('draft', 'partial_failed')"
        )
        .bind(user_id)
        .fetch_one(db)
        .await?
    }
    ThreadStatusFilter::Posted => {
        sqlx::query_as(
            "SELECT COUNT(*) FROM tweet_threads WHERE user_id = $1 AND status = 'posted'"
        )
        .bind(user_id)
        .fetch_one(db)
        .await?
    }
    ThreadStatusFilter::All => {
        sqlx::query_as(
            "SELECT COUNT(*) FROM tweet_threads WHERE user_id = $1"
        )
        .bind(user_id)
        .fetch_one(db)
        .await?
    }
};
```

**Why It's Bad:**
- Same match pattern in `list_threads()` and `list_threads_paginated()`
- 80+ lines of near-identical code
- WHERE conditions are the only difference

**Suggested Fix:**

```rust
fn thread_status_condition(filter: ThreadStatusFilter) -> &'static str {
    match filter {
        ThreadStatusFilter::Pending => "AND status IN ('draft', 'posting', 'partial_failed')",
        ThreadStatusFilter::Posted => "AND status = 'posted'",
        ThreadStatusFilter::All => "",
    }
}
```

**Impact:** ~80 lines of duplication removed

---

### Finding 3: Duplicated Token Refresh Logic

**Files:**
- `api/src/routes/content/twitter/tweets.rs` (lines 113-144)
- `api/src/routes/content/twitter/tweets.rs` (lines 321-346 in `do_publish_with_progress`)
- `api/src/routes/content/twitter/threads.rs` (lines 368-399)

**The Problem:**

Same 33-line token refresh block appears 3 times:

```rust
let access_token = if tokens.token_expires_at < Utc::now() {
    if let Some(refresh_token) = &tokens.refresh_token {
        let new_tokens = state
            .twitter
            .refresh_token(refresh_token)
            .await
            .map_err(|e| {
                eprintln!("Token refresh error: {}", e);
                StatusCode::UNAUTHORIZED
            })?;

        let expires_at = Utc::now() + Duration::seconds(new_tokens.expires_in);
        twitter::update_user_tokens(
            &state.db,
            user_id,
            &new_tokens.access_token,
            new_tokens.refresh_token.as_deref(),
            expires_at,
        )
        .await
        .map_err(|e| {
            eprintln!("Update user tokens error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?;

        new_tokens.access_token
    } else {
        return Err(StatusCode::UNAUTHORIZED);
    }
} else {
    tokens.access_token
};
```

**Why It's Bad:**
- 99 total lines of identical code (33 × 3)
- Bug fixes must be applied 3 times
- Different error types in different locations (StatusCode vs String)

**Suggested Fix:**

```rust
// services/auth.rs
pub async fn ensure_valid_access_token(
    state: &Arc<AppState>,
    user_id: i64,
    tokens: UserTokens,
) -> Result<String, StatusCode> {
    if tokens.token_expires_at >= Utc::now() {
        return Ok(tokens.access_token);
    }

    let refresh_token = tokens.refresh_token.ok_or(StatusCode::UNAUTHORIZED)?;

    let new_tokens = state.twitter.refresh_token(&refresh_token).await
        .map_err(|e| { eprintln!("Token refresh error: {}", e); StatusCode::UNAUTHORIZED })?;

    let expires_at = Utc::now() + Duration::seconds(new_tokens.expires_in);
    twitter::update_user_tokens(&state.db, user_id, &new_tokens.access_token,
        new_tokens.refresh_token.as_deref(), expires_at).await
        .map_err(|e| { eprintln!("Update tokens error: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?;

    Ok(new_tokens.access_token)
}

// Usage becomes one line:
let access_token = ensure_valid_access_token(&state, user_id, tokens).await?;
```

**Impact:** 99 lines → 15 lines (84 lines removed)

---

### Finding 4: Verbose Error Handling Boilerplate

**File:** `api/src/routes/content/twitter/tweets.rs`
**Lines:** 86-193 (`post_tweet` function)

**The Problem:**

Nearly every operation has the same error handling pattern:

```rust
let tweet = tweets::get_tweet_for_posting(&state.db, tweet_collateral_id, user_id)
    .await
    .map_err(|e| {
        eprintln!("Get tweet for posting error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::NOT_FOUND)?;

let tokens = twitter::get_user_tokens(&state.db, user_id)
    .await
    .map_err(|e| {
        eprintln!("Get user tokens error: {}", e);
        StatusCode::INTERNAL_SERVER_ERROR
    })?
    .ok_or(StatusCode::UNAUTHORIZED)?;

// ... repeats ~8 more times
```

**Why It's Bad:**
- Same `.map_err(|e| { eprintln!(...); StatusCode::INTERNAL_SERVER_ERROR })?` pattern 8+ times
- 50% of the function is error handling boilerplate
- Different error messages serve no real purpose (not user-facing)

**Suggested Fix:**

```rust
// Create a trait extension
trait LogErrorAsStatus<T> {
    fn log_as_500(self, context: &str) -> Result<T, StatusCode>;
}

impl<T, E: std::fmt::Display> LogErrorAsStatus<T> for Result<T, E> {
    fn log_as_500(self, context: &str) -> Result<T, StatusCode> {
        self.map_err(|e| {
            eprintln!("{}: {}", context, e);
            StatusCode::INTERNAL_SERVER_ERROR
        })
    }
}

// Usage:
let tweet = tweets::get_tweet_for_posting(&state.db, tweet_collateral_id, user_id)
    .await
    .log_as_500("get_tweet_for_posting")?
    .ok_or(StatusCode::NOT_FOUND)?;
```

Or use a macro:
```rust
macro_rules! db_500 {
    ($expr:expr) => {
        $expr.await.map_err(|e| {
            eprintln!("DB error: {}", e);
            StatusCode::INTERNAL_SERVER_ERROR
        })?
    };
}
```

**Impact:** ~40 lines of boilerplate removed per route handler

---

### Finding 5: Tuple-to-Struct Boilerplate

**File:** `api/src/domain/content.rs`
**Lines:** 229-263

**The Problem:**

Fetching as tuple then manually converting to struct:

```rust
let thread_rows: Vec<(
    i64,
    i64,
    Option<String>,
    String,
    DateTime<Utc>,
    Option<DateTime<Utc>>,
    Option<String>,
)> = sqlx::query_as(
    r#"
    SELECT id, user_id, title, status, created_at, posted_at, first_tweet_id
    FROM tweet_threads
    WHERE id = ANY($1) AND user_id = $2
    "#,
)
.bind(&thread_ids)
.bind(user_id)
.fetch_all(db)
.await?;

// Then convert tuple to struct
let thread_structs: Vec<TweetThread> = thread_rows
    .into_iter()
    .map(
        |(id, user_id, title, status, created_at, posted_at, first_tweet_id)| TweetThread {
            id,
            user_id,
            title,
            status: ThreadStatus::from_str(&status),
            created_at,
            posted_at,
            first_tweet_id,
        },
    )
    .collect();
```

**Why It's Bad:**
- 34 lines for what could be 5 lines
- Error-prone tuple field ordering
- TweetThread already has `#[derive(Debug)]` but not `sqlx::FromRow`

**Suggested Fix:**

Add `#[derive(sqlx::FromRow)]` to TweetThread (with status as String, convert at use site):

```rust
let thread_structs: Vec<TweetThread> = sqlx::query_as(
    "SELECT id, user_id, title, status, created_at, posted_at, first_tweet_id
     FROM tweet_threads WHERE id = ANY($1) AND user_id = $2"
)
.bind(&thread_ids)
.bind(user_id)
.fetch_all(db)
.await?;
```

**Impact:** 34 lines → 8 lines

---

### Finding 6: Repeated Clone Pattern in Agent

**File:** `api/src/agent.rs`
**Lines:** 406-407, 416-417, 463-465, 488-489, 577-578

**The Problem:**

Clone context and media for each tool closure:

```rust
let ctx = context.clone();
let media_for_tool = uploaded_media.clone();

// Then in closure
let ctx = ctx.clone();
let media = media_for_tool.clone();

// Repeated 5+ times for each tool registration
```

**Why It's Bad:**
- 20+ lines of repetitive cloning
- Creates cognitive load ("why so many clones?")
- Each tool has nearly identical setup

**Impact:** ~20 lines of repetitive code

---

### Finding 7: String Status Comparisons

**File:** `api/src/routes/content/twitter/threads.rs`
**Lines:** 160-170, 240-250, 283-293, 330-340

**The Problem:**

Status fetched as string, compared to string literals:

```rust
let status = threads::get_thread_status(&state.db, thread_id, user_id)
    .await
    .map_err(|e| { eprintln!("Get thread status error: {}", e); StatusCode::INTERNAL_SERVER_ERROR })?
    .ok_or(StatusCode::NOT_FOUND)?;

if status != "draft" {
    return Err(StatusCode::CONFLICT);
}
```

**Why It's Bad:**
- String comparison instead of enum (typo risk: "Draft" vs "draft")
- Pattern repeated 4 times
- Domain defines `ThreadStatus` enum but routes don't use it

**Suggested Fix:**

Return `ThreadStatus` enum from domain function, compare with `ThreadStatus::Draft`.

**Impact:** Type safety + 4 repeated validations consolidated

---

## Part 2: TypeScript/Lit Frontend (web/src/)

### Finding 8: Repeated Error Handling Pattern

**Files:**
- `web/src/components/tweet-card.ts` (lines 56-58)
- `web/src/components/thread-card.ts` (lines 32-34, 50-51)
- `web/src/components/login-page.ts` (lines 21-22)
- `web/src/components/dashboard-page.ts` (lines 134-136, 150-151, 173-174, 191-192)
- `web/src/components/media-browser.ts` (lines 127-128, 145-146, 223-224)

**The Problem:**

Same error extraction pattern repeated 8+ times:

```typescript
catch (e) {
  console.error('Failed to post tweet:', e);
  this.error = e instanceof Error ? e.message : 'Failed to post tweet';
}
```

**Why It's Bad:**
- 8+ duplicate patterns
- If error handling logic changes, must update all locations

**Suggested Fix:**

```typescript
// utils/error.ts
export function getErrorMessage(e: unknown, fallback: string): string {
  return e instanceof Error ? e.message : fallback;
}

// Usage:
catch (e) {
  console.error('Failed to post tweet:', e);
  this.error = getErrorMessage(e, 'Failed to post tweet');
}
```

**Impact:** Consistent error handling, single source of truth

---

### Finding 9: Nested Loading/Error/Content Ternaries

**Files:**
- `web/src/components/media-browser.ts` (lines 304-341)
- `web/src/components/dashboard-page.ts` (lines 251-276)
- `web/src/components/tweet-content.ts` (lines 114-153)

**The Problem:**

```typescript
${this.loading
  ? html`
      <div class="flex justify-center py-8">
        <span class="loading loading-spinner loading-md"></span>
      </div>
    `
  : this.loadError
  ? html`
      <div class="flex flex-col items-center justify-center py-8 text-center">
        <svg class="w-10 h-10 text-error mb-2" ...>...</svg>
        <p class="text-error text-sm">${this.loadError}</p>
        <button class="btn btn-sm btn-ghost mt-2" @click=${() => this.loadCaptures()}>Retry</button>
      </div>
    `
  : html`...actual content...`}
```

**Why It's Bad:**
- Deep nesting with large template blocks
- Same error icon SVG duplicated
- Pattern repeated across 3 components

**Suggested Fix:**

```typescript
private renderLoading() {
  return html`<div class="flex justify-center py-8">
    <span class="loading loading-spinner loading-md"></span>
  </div>`;
}

private renderError(error: string, onRetry: () => void) {
  return html`<div class="flex flex-col items-center justify-center py-8 text-center">
    ${ErrorIcon('w-10 h-10 text-error mb-2')}
    <p class="text-error text-sm">${error}</p>
    <button class="btn btn-sm btn-ghost mt-2" @click=${onRetry}>Retry</button>
  </div>`;
}
```

**Impact:** ~50 lines across 3 components simplified

---

### Finding 10: Duplicated Error Icon SVG

**Files:** 7 locations across login-page.ts, dashboard-page.ts, tweet-card.ts, thread-card.ts, tweet-content.ts, media-browser.ts

**The Problem:**

Exact same SVG inlined 7 times:

```typescript
<svg xmlns="http://www.w3.org/2000/svg" class="stroke-current shrink-0 h-5 w-5" fill="none" viewBox="0 0 24 24">
  <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2" d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
</svg>
```

**Suggested Fix:**

```typescript
// components/icons.ts
export const ErrorIcon = (classes = 'stroke-current shrink-0 h-5 w-5') => html`
  <svg class="${classes}" fill="none" stroke="currentColor" viewBox="0 0 24 24">
    <path stroke-linecap="round" stroke-linejoin="round" stroke-width="2"
      d="M10 14l2-2m0 0l2-2m-2 2l-2-2m2 2l2 2m7-2a9 9 0 11-18 0 9 9 0 0118 0z" />
  </svg>
`;
```

**Impact:** ~21 lines of duplicated SVG removed

---

### Finding 11: Complex Image Grid Layout

**File:** `web/src/components/tweet-content.ts`
**Lines:** 216-289

**The Problem:**

74 lines with 4 separate if-branches for image counts 1, 2, 3, 4+:

```typescript
if (count === 1) {
  return html`<img src=${this.imageUrls[0].url} class="..." />`;
}

if (count === 2) {
  return html`
    <div class="grid grid-rows-2 h-full gap-0.5">
      ${this.imageUrls.map((img) => html`<img src=${img.url} ... />`)}
    </div>
  `;
}

if (count === 3) {
  return html`
    <div class="grid grid-cols-2 h-full gap-0.5">
      <img src=${this.imageUrls[0].url} ... />
      <img src=${this.imageUrls[1].url} ... />
      <img src=${this.imageUrls[2].url} ... />
    </div>
  `;
}

// 4+ case...
```

**Why It's Bad:**
- Repeats image rendering logic 4 times
- Each branch has same click handler, same styling
- Hard to maintain grid classes

**Suggested Fix:**

```typescript
private getGridClass(count: number): string {
  if (count === 1) return '';
  if (count === 2) return 'grid-rows-2';
  if (count === 3) return 'grid-cols-2';
  return 'grid-cols-2 grid-rows-2';
}

private renderImageGrid() {
  const count = this.imageUrls.length;
  return html`
    <div class="grid h-full gap-0.5 ${this.getGridClass(count)}">
      ${this.imageUrls.slice(0, 4).map((img, i) => html`
        <img src=${img.url}
          class="w-full h-full object-cover cursor-pointer ${count === 3 && i === 0 ? 'row-span-2' : ''}"
          @click=${() => this.openFullscreen(img.url)} />
      `)}
    </div>
  `;
}
```

**Impact:** 74 lines → ~25 lines (66% reduction)

---

### Finding 12: Nested Post Button State

**File:** `web/src/components/tweet-card.ts`
**Lines:** 152-172

**The Problem:**

4-level nested ternary:

```typescript
${this.posting
  ? this.uploadStatus === 'uploading' && this.uploadProgress !== null
    ? html`<div class="flex items-center gap-2">...</div>`
    : this.uploadStatus === 'processing'
      ? html`<span class="loading ..."></span><span>Processing...</span>`
      : this.uploadStatus === 'posting'
        ? html`<span class="loading ..."></span><span>Posting...</span>`
        : html`<span class="loading ..."></span>`
  : html`...Post button...`}
```

**Suggested Fix:**

Extract to method with clear if-else:

```typescript
private renderPostButtonContent() {
  if (!this.posting) return html`<svg ...></svg> Post`;

  if (this.uploadStatus === 'uploading' && this.uploadProgress !== null) {
    return html`<div class="radial-progress ..." style="--value:${this.uploadProgress}">...</div>`;
  }

  const text = this.uploadStatus === 'processing' ? 'Processing...' : 'Posting...';
  return html`<span class="loading loading-spinner loading-sm"></span><span class="text-xs">${text}</span>`;
}
```

**Impact:** Readable state machine vs nested ternary

---

### Finding 13: Verbose Query String Building

**File:** `web/src/api.ts`
**Lines:** 507-517, 591-598

**The Problem:**

```typescript
const query = new URLSearchParams();
if (params.start) query.set('start', params.start);
if (params.end) query.set('end', params.end);
if (params.type) query.set('type', params.type);
if (params.limit) query.set('limit', params.limit.toString());
if (params.offset) query.set('offset', params.offset.toString());
if (params.include_ids?.length) query.set('include_ids', params.include_ids.join(','));

const url = `${API_BASE}/captures/browse${query.toString() ? '?' + query.toString() : ''}`;
```

**Suggested Fix:**

```typescript
private buildUrl(path: string, params: Record<string, unknown>): string {
  const query = new URLSearchParams();
  Object.entries(params).forEach(([key, value]) => {
    if (value != null) {
      query.set(key, Array.isArray(value) ? value.join(',') : String(value));
    }
  });
  const qs = query.toString();
  return qs ? `${path}?${qs}` : path;
}

// Usage:
const url = this.buildUrl(`${API_BASE}/captures/browse`, params);
```

**Impact:** Reusable, concise query building

---

## Summary

| Category | Lines Removed/Simplified | Priority |
|----------|--------------------------|----------|
| SQL match arm duplication (tweets.rs, threads.rs) | ~170 lines | High |
| Token refresh duplication | ~84 lines | High |
| Error handling boilerplate (Rust) | ~40+ lines per handler | High |
| Error icon duplication (Frontend) | ~21 lines | Medium |
| Image grid simplification | ~50 lines | Medium |
| Loading/error/content ternaries | ~50 lines | Medium |
| Tuple-to-struct boilerplate | ~26 lines | Low |
| Query string building | ~15 lines | Low |

**Total Estimated Impact:** 400-500 lines of unnecessary code that could be removed or simplified.

---

## Recommendations

### Immediate Actions (High Impact)
1. Extract `ensure_valid_access_token()` helper for token refresh
2. Create `status_where_clause()` helper for SQL filter patterns
3. Create error icon and loading state helpers in frontend

### Medium-Term (Code Quality)
4. Add trait extension for error handling in Rust routes
5. Consolidate image grid rendering logic
6. Extract URL query building helper

### Consider But Evaluate
7. Adding `sqlx::FromRow` to more structs (evaluate compile-time cost)
8. Using macros for repetitive patterns (evaluate readability tradeoff)
