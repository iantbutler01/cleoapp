use base64::Engine;
use chrono::{DateTime, Duration, Utc};
use google_cloud_storage::client::Storage;
use reson_agentic::Tool;
use reson_agentic::agentic;
use reson_agentic::providers::{FileState, GoogleGenAIClient};
use reson_agentic::runtime::ToolFunction;
use reson_agentic::types::{
    ChatMessage, ChatRole, CreateResult, MediaPart, MediaSource, MultimodalMessage, ToolCall,
    ToolResult,
};
use reson_agentic::utils::ConversationMessage;
use serde::{Deserialize, Serialize};
use sqlx::PgPool;
use std::sync::Arc;
use tokio::sync::Mutex;

use crate::constants::BUCKET_NAME;

const MAX_TURNS: usize = 40;

// Tool definitions

/// Write a tweet with optional media attachments. Use this when you find something tweet-worthy.
#[derive(Tool, Serialize, Deserialize, Debug)]
pub struct WriteTweet {
    /// The tweet text content (max 280 chars)
    pub text: String,
    /// Video timestamp to clip from (e.g., "0:30") - optional
    pub video_timestamp: Option<String>,
    /// Duration of video clip in seconds (default 10) - optional
    pub video_duration: Option<u32>,
    /// Capture IDs to attach as images - optional
    pub image_capture_ids: Option<Vec<i64>>,
    /// Why this moment is tweet-worthy
    pub rationale: String,
}

/// Signal that you've finished reviewing all interesting content in this time window.
#[derive(Tool, Serialize, Deserialize, Debug)]
pub struct MarkComplete {
    /// Brief summary of what was found
    pub summary: String,
    /// Number of tweets generated
    pub tweets_generated: u32,
}

/// Request more context about a specific time range or capture
#[derive(Tool, Serialize, Deserialize, Debug)]
pub struct GetMoreContext {
    /// Start of time range to examine
    pub start_time: String,
    /// End of time range to examine
    pub end_time: String,
    /// What you're looking for
    pub query: String,
}

/// A single tweet within a thread
#[derive(Serialize, Deserialize, Debug, Clone)]
pub struct ThreadTweetInput {
    /// Tweet text (max 280 chars)
    pub text: String,
    /// Capture IDs to attach as images
    pub image_capture_ids: Option<Vec<i64>>,
    /// Video timestamp to clip from (e.g., "0:30")
    pub video_timestamp: Option<String>,
    /// Duration of video clip in seconds
    pub video_duration: Option<u32>,
}

/// Create a tweet thread (multiple tweets posted as a reply chain).
/// Use this when content deserves deeper exploration than a single tweet.
#[derive(Tool, Serialize, Deserialize, Debug)]
pub struct WriteThread {
    /// Optional title for the thread (for user organization, not posted)
    pub title: Option<String>,
    /// The tweets in order. First tweet posts normally, rest reply to previous.
    pub tweets: Vec<ThreadTweetInput>,
    /// Why this is thread-worthy (vs individual tweets)
    pub rationale: String,
}

// Collateral output types

#[derive(Debug, Clone, Serialize)]
pub struct TweetCollateral {
    pub text: String,
    pub video_clip: Option<VideoClip>,
    pub image_capture_ids: Vec<i64>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
    /// Thread ID if this tweet belongs to a thread
    pub thread_id: Option<i64>,
    /// Position in thread (0-indexed)
    pub thread_position: Option<i32>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoClip {
    pub source_capture_id: i64,
    pub start_timestamp: String,
    pub duration_secs: u32,
}

#[derive(Debug, Clone)]
pub struct ThreadMetadata {
    pub id: i64,
    pub title: Option<String>,
    #[allow(dead_code)]
    pub tweet_count: usize,
}

#[derive(Debug)]
pub struct AgentContext {
    pub db: PgPool,
    #[allow(dead_code)]
    pub gcs: Storage,
    pub user_id: i64,
    pub window_start: DateTime<Utc>,
    pub window_end: DateTime<Utc>,
    pub tweets: Vec<TweetCollateral>,
    pub threads: Vec<ThreadMetadata>,
    pub completed: bool,
    /// Counter for generating thread IDs within a run
    pub next_thread_id: i64,
}

// Data fetching

#[derive(Debug, sqlx::FromRow)]
pub struct CaptureRecord {
    pub id: i64,
    pub media_type: String,
    pub content_type: String,
    pub gcs_path: String,
    pub captured_at: DateTime<Utc>,
}

#[derive(Debug, sqlx::FromRow)]
pub struct ActivityRecord {
    #[allow(dead_code)]
    pub id: i64,
    pub timestamp: DateTime<Utc>,
    pub event_type: String,
    pub application: Option<String>,
    pub window: Option<String>,
}

/// Maximum captures to fetch for agent context (prevents OOM on large time windows)
const MAX_AGENT_CAPTURES: i64 = 100;
/// Maximum activities to fetch for agent context
const MAX_AGENT_ACTIVITIES: i64 = 500;

pub async fn fetch_captures_in_window(
    db: &PgPool,
    user_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<CaptureRecord>, sqlx::Error> {
    sqlx::query_as::<_, CaptureRecord>(
        r#"
        SELECT id, media_type, content_type, gcs_path, captured_at
        FROM captures
        WHERE user_id = $1 AND captured_at >= $2 AND captured_at < $3
        ORDER BY captured_at ASC
        LIMIT $4
        "#,
    )
    .bind(user_id)
    .bind(start)
    .bind(end)
    .bind(MAX_AGENT_CAPTURES)
    .fetch_all(db)
    .await
}

pub async fn fetch_activities_in_window(
    db: &PgPool,
    user_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ActivityRecord>, sqlx::Error> {
    sqlx::query_as::<_, ActivityRecord>(
        r#"
        SELECT id, timestamp, event_type, application, "window"
        FROM activities
        WHERE user_id = $1 AND timestamp >= $2 AND timestamp < $3
        ORDER BY timestamp ASC
        LIMIT $4
        "#,
    )
    .bind(user_id)
    .bind(start)
    .bind(end)
    .bind(MAX_AGENT_ACTIVITIES)
    .fetch_all(db)
    .await
}

pub async fn get_last_run_time(db: &PgPool, user_id: i64) -> Option<DateTime<Utc>> {
    sqlx::query_scalar::<_, DateTime<Utc>>(
        r#"
        SELECT completed_at FROM agent_runs
        WHERE user_id = $1
        ORDER BY completed_at DESC
        LIMIT 1
        "#,
    )
    .bind(user_id)
    .fetch_optional(db)
    .await
    .ok()
    .flatten()
}

/// Find users who:
/// 1. Have no activity in the last `idle_minutes`
/// 2. Have captures that haven't been processed (captured after last agent run)
pub async fn find_idle_users_with_pending_captures(
    db: &PgPool,
    idle_minutes: i64,
) -> Result<Vec<i64>, sqlx::Error> {
    let idle_threshold = Utc::now() - Duration::minutes(idle_minutes);

    sqlx::query_scalar::<_, i64>(
        r#"
        SELECT DISTINCT c.user_id
        FROM captures c
        LEFT JOIN agent_runs ar ON ar.user_id = c.user_id
        WHERE
            -- Has captures after last run (or never ran)
            c.captured_at > COALESCE(
                (SELECT MAX(completed_at) FROM agent_runs WHERE user_id = c.user_id),
                '1970-01-01'::timestamptz
            )
            -- No recent activity (user is idle)
            AND NOT EXISTS (
                SELECT 1 FROM captures c2
                WHERE c2.user_id = c.user_id
                AND c2.captured_at > $1
            )
        "#,
    )
    .bind(idle_threshold)
    .fetch_all(db)
    .await
}

pub async fn record_run(
    db: &PgPool,
    user_id: i64,
    window_start: DateTime<Utc>,
    window_end: DateTime<Utc>,
    tweets_generated: i32,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO agent_runs (user_id, window_start, window_end, tweets_generated, completed_at)
        VALUES ($1, $2, $3, $4, NOW())
        "#,
    )
    .bind(user_id)
    .bind(window_start)
    .bind(window_end)
    .bind(tweets_generated)
    .execute(db)
    .await?;
    Ok(())
}

#[allow(dead_code)] // Kept for reference; save_threads_and_tweets is now preferred
pub async fn save_tweet(
    db: &PgPool,
    user_id: i64,
    tweet: &TweetCollateral,
    thread_id_map: &std::collections::HashMap<i64, i64>, // Maps temp ID -> real DB ID
) -> Result<i64, sqlx::Error> {
    let video_clip_json = tweet
        .video_clip
        .as_ref()
        .map(|c| serde_json::to_value(c).unwrap());
    let image_ids: Vec<i64> = tweet.image_capture_ids.clone();

    // Map temp thread_id to real DB thread_id
    let real_thread_id = tweet.thread_id.and_then(|tid| thread_id_map.get(&tid).copied());

    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, thread_id, thread_position)
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&tweet.text)
    .bind(video_clip_json)
    .bind(&image_ids)
    .bind(&tweet.rationale)
    .bind(tweet.created_at)
    .bind(real_thread_id)
    .bind(tweet.thread_position)
    .fetch_one(db)
    .await?;

    Ok(row.0)
}

#[allow(dead_code)] // Kept for reference; save_threads_and_tweets is now preferred
pub async fn save_thread(
    db: &PgPool,
    user_id: i64,
    thread: &ThreadMetadata,
) -> Result<i64, sqlx::Error> {
    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO tweet_threads (user_id, title, status, created_at)
        VALUES ($1, $2, 'draft', NOW())
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&thread.title)
    .fetch_one(db)
    .await?;
    Ok(row.0)
}

/// Save threads and tweets atomically in a transaction
/// If any tweet fails to save, all threads and tweets are rolled back
pub async fn save_threads_and_tweets(
    db: &PgPool,
    user_id: i64,
    threads: &[ThreadMetadata],
    tweets: &[TweetCollateral],
) -> Result<(), sqlx::Error> {
    let mut tx = db.begin().await?;

    // Save threads first and build mapping from temp ID -> real DB ID
    let mut thread_id_map = std::collections::HashMap::new();
    for thread in threads {
        let row: (i64,) = sqlx::query_as(
            r#"
            INSERT INTO tweet_threads (user_id, title, status, created_at)
            VALUES ($1, $2, 'draft', NOW())
            RETURNING id
            "#,
        )
        .bind(user_id)
        .bind(&thread.title)
        .fetch_one(&mut *tx)
        .await?;
        thread_id_map.insert(thread.id, row.0);
    }

    // Save tweets
    for tweet in tweets {
        let video_clip_json = tweet
            .video_clip
            .as_ref()
            .map(|c| serde_json::to_value(c).unwrap());
        let image_ids: Vec<i64> = tweet.image_capture_ids.clone();
        let real_thread_id = tweet.thread_id.and_then(|tid| thread_id_map.get(&tid).copied());

        sqlx::query(
            r#"
            INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at, thread_id, thread_position)
            VALUES ($1, $2, $3, $4, $5, $6, $7, $8)
            "#,
        )
        .bind(user_id)
        .bind(&tweet.text)
        .bind(video_clip_json)
        .bind(&image_ids)
        .bind(&tweet.rationale)
        .bind(tweet.created_at)
        .bind(real_thread_id)
        .bind(tweet.thread_position)
        .execute(&mut *tx)
        .await?;
    }

    tx.commit().await?;
    Ok(())
}

// Agent implementation

// Uploaded media reference
#[derive(Clone)]
pub struct UploadedMedia {
    pub capture_id: i64,
    pub uri: String,
    pub mime_type: String,
    pub is_video: bool,
}

#[agentic(model = "gemini:gemini-3.0-flash")]
pub async fn run_collateral_agent(
    context: Arc<Mutex<AgentContext>>,
    captures: Vec<CaptureRecord>,
    activities: Vec<ActivityRecord>,
    uploaded_media: Vec<UploadedMedia>, // Videos (File API) + Images (File API or base64)
    runtime: Runtime,
) -> reson_agentic::error::Result<()> {
    let ctx = context.clone();
    let media_for_tool = uploaded_media.clone();

    // Register WriteTweet tool
    runtime
        .register_tool_with_schema(
            WriteTweet::tool_name(),
            WriteTweet::description(),
            WriteTweet::schema(),
            ToolFunction::Async(Box::new({
                let ctx = ctx.clone();
                let media = media_for_tool.clone();
                move |args| {
                    let ctx = ctx.clone();
                    let media = media.clone();
                    Box::pin(async move {
                        println!("[agent] WriteTweet tool called with args: {:?}", args);
                        let tweet: WriteTweet = serde_json::from_value(args)?;
                        let mut guard = ctx.lock().await;

                        // Find video capture if timestamp provided
                        let video_clip = if let Some(ts) = &tweet.video_timestamp {
                            // Find first video in uploaded media
                            media.iter().find(|m| m.is_video).map(|m| VideoClip {
                                source_capture_id: m.capture_id,
                                start_timestamp: ts.clone(),
                                duration_secs: tweet.video_duration.unwrap_or(10),
                            })
                        } else {
                            None
                        };

                        let collateral = TweetCollateral {
                            text: tweet.text.clone(),
                            video_clip,
                            image_capture_ids: tweet.image_capture_ids.unwrap_or_default(),
                            rationale: tweet.rationale.clone(),
                            created_at: Utc::now(),
                            thread_id: None,
                            thread_position: None,
                        };

                        guard.tweets.push(collateral);
                        Ok(format!("Tweet saved: {}", tweet.text))
                    })
                }
            })),
        )
        .await?;

    // Register MarkComplete tool
    runtime
        .register_tool_with_schema(
            MarkComplete::tool_name(),
            MarkComplete::description(),
            MarkComplete::schema(),
            ToolFunction::Async(Box::new({
                let ctx = ctx.clone();
                move |args| {
                    let ctx = ctx.clone();
                    Box::pin(async move {
                        println!("[agent] MarkComplete tool called with args: {:?}", args);
                        let complete: MarkComplete = serde_json::from_value(args)?;
                        let mut guard = ctx.lock().await;
                        guard.completed = true;
                        Ok(format!(
                            "Marked complete. Summary: {}. Tweets: {}",
                            complete.summary, complete.tweets_generated
                        ))
                    })
                }
            })),
        )
        .await?;

    // Register GetMoreContext tool
    runtime
        .register_tool_with_schema(
            GetMoreContext::tool_name(),
            GetMoreContext::description(),
            GetMoreContext::schema(),
            ToolFunction::Async(Box::new({
                let ctx = ctx.clone();
                move |args| {
                    let ctx = ctx.clone();
                    Box::pin(async move {
                        println!("[agent] GetMoreContext tool called with args: {:?}", args);
                        let request: GetMoreContext = serde_json::from_value(args)?;
                        let guard = ctx.lock().await;

                        // Parse time strings (expect HH:MM or HH:MM:SS format)
                        let parse_time = |s: &str, base_date: DateTime<Utc>| -> Option<DateTime<Utc>> {
                            let parts: Vec<&str> = s.split(':').collect();
                            if parts.len() >= 2 {
                                let hour: u32 = parts[0].parse().ok()?;
                                let min: u32 = parts[1].parse().ok()?;
                                let sec: u32 = parts.get(2).and_then(|s| s.parse().ok()).unwrap_or(0);
                                Some(base_date.date_naive().and_hms_opt(hour, min, sec)?.and_utc())
                            } else {
                                None
                            }
                        };

                        let base_date = guard.window_start;
                        let start = parse_time(&request.start_time, base_date)
                            .unwrap_or(guard.window_start);
                        let end = parse_time(&request.end_time, base_date)
                            .unwrap_or(guard.window_end);

                        // Fetch more detailed activities in the requested range
                        let activities = fetch_activities_in_window(&guard.db, guard.user_id, start, end)
                            .await
                            .unwrap_or_default();

                        // Fetch captures in the requested range
                        let captures = fetch_captures_in_window(&guard.db, guard.user_id, start, end)
                            .await
                            .unwrap_or_default();

                        // Build detailed response
                        let activity_details: String = activities
                            .iter()
                            .map(|a| {
                                format!(
                                    "[{}] {}: {} - {}",
                                    a.timestamp.format("%H:%M:%S"),
                                    a.event_type,
                                    a.application.as_deref().unwrap_or("unknown"),
                                    a.window.as_deref().unwrap_or("")
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        let capture_details: String = captures
                            .iter()
                            .map(|c| {
                                format!(
                                    "[{}] {} (id: {}, path: {})",
                                    c.captured_at.format("%H:%M:%S"),
                                    c.media_type,
                                    c.id,
                                    c.gcs_path
                                )
                            })
                            .collect::<Vec<_>>()
                            .join("\n");

                        Ok(format!(
                            "Context for '{}' ({} to {}):\n\nACTIVITIES ({} events):\n{}\n\nCAPTURES ({} files):\n{}",
                            request.query,
                            start.format("%H:%M:%S"),
                            end.format("%H:%M:%S"),
                            activities.len(),
                            if activity_details.is_empty() { "None" } else { &activity_details },
                            captures.len(),
                            if capture_details.is_empty() { "None" } else { &capture_details }
                        ))
                    })
                }
            })),
        )
        .await?;

    // Register WriteThread tool
    runtime
        .register_tool_with_schema(
            WriteThread::tool_name(),
            WriteThread::description(),
            WriteThread::schema(),
            ToolFunction::Async(Box::new({
                let ctx = ctx.clone();
                let media = media_for_tool.clone();
                move |args| {
                    let ctx = ctx.clone();
                    let media = media.clone();
                    Box::pin(async move {
                        println!("[agent] WriteThread tool called with args: {:?}", args);
                        let thread: WriteThread = serde_json::from_value(args)?;
                        let mut guard = ctx.lock().await;

                        if thread.tweets.is_empty() {
                            return Ok("Error: Thread must have at least one tweet".to_string());
                        }

                        // Generate thread ID (will be replaced with real DB ID when saved)
                        let thread_id = guard.next_thread_id;
                        guard.next_thread_id += 1;

                        // Convert each tweet input to TweetCollateral with thread info
                        for (position, tweet_input) in thread.tweets.iter().enumerate() {
                            let video_clip = if let Some(ts) = &tweet_input.video_timestamp {
                                media.iter().find(|m| m.is_video).map(|m| VideoClip {
                                    source_capture_id: m.capture_id,
                                    start_timestamp: ts.clone(),
                                    duration_secs: tweet_input.video_duration.unwrap_or(10),
                                })
                            } else {
                                None
                            };

                            let collateral = TweetCollateral {
                                text: tweet_input.text.clone(),
                                video_clip,
                                image_capture_ids: tweet_input.image_capture_ids.clone().unwrap_or_default(),
                                rationale: thread.rationale.clone(),
                                created_at: Utc::now(),
                                thread_id: Some(thread_id),
                                thread_position: Some(position as i32),
                            };
                            guard.tweets.push(collateral);
                        }

                        // Store thread metadata
                        guard.threads.push(ThreadMetadata {
                            id: thread_id,
                            title: thread.title.clone(),
                            tweet_count: thread.tweets.len(),
                        });

                        Ok(format!(
                            "Thread created with {} tweets{}",
                            thread.tweets.len(),
                            thread.title.as_ref().map(|t| format!(": {}", t)).unwrap_or_default()
                        ))
                    })
                }
            })),
        )
        .await?;

    // Build activity summary
    let activity_summary: String = activities
        .iter()
        .take(50) // Limit for context window
        .map(|a| {
            format!(
                "[{}] {}: {} - {}",
                a.timestamp.format("%H:%M:%S"),
                a.event_type,
                a.application.as_deref().unwrap_or("unknown"),
                a.window.as_deref().unwrap_or("")
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Build capture summary
    let capture_summary: String = captures
        .iter()
        .map(|c| {
            format!(
                "[{}] {} ({})",
                c.captured_at.format("%H:%M:%S"),
                c.media_type,
                c.id
            )
        })
        .collect::<Vec<_>>()
        .join("\n");

    // Build multimodal message with all media
    let mut parts: Vec<MediaPart> = Vec::new();

    // Add media parts (videos via FileUri, images via base64)
    for media in &uploaded_media {
        if media.is_video {
            parts.push(MediaPart::Video {
                source: MediaSource::FileUri {
                    uri: media.uri.clone(),
                    mime_type: Some(media.mime_type.clone()),
                },
                metadata: None,
            });
        } else {
            // Images are base64 encoded in the uri field
            parts.push(MediaPart::Image {
                source: MediaSource::Base64 {
                    data: media.uri.clone(),
                    mime_type: media.mime_type.clone(),
                },
                detail: None,
            });
        }
    }

    // Extract window timestamps once to avoid multiple lock acquisitions
    let (window_start_str, window_end_str) = {
        let guard = ctx.lock().await;
        (
            guard.window_start.format("%Y-%m-%d %H:%M").to_string(),
            guard.window_end.format("%Y-%m-%d %H:%M").to_string(),
        )
    };

    // Add text prompt
    let prompt = format!(
        r#"You are reviewing captured screen recordings and activity data to find tweet-worthy moments.

TIME WINDOW: {} to {}

ACTIVITY LOG:
{}

CAPTURES AVAILABLE:
{}

Your job is to:
1. Review the videos and activity data
2. Find interesting, funny, or notable moments
3. Decide whether content deserves a single tweet or a thread:
   - Use WriteTweet for standalone moments (quick wins, single observations)
   - Use WriteThread for narrative arcs (debugging journeys, feature builds, learning progressions)
4. When done reviewing, use MarkComplete

THREAD GUIDELINES:
- Threads should tell a story with a beginning, middle, and end
- Each tweet in a thread should stand alone but connect to the narrative
- Good thread length: 3-7 tweets (not too short, not overwhelming)
- First tweet should hook the reader
- Last tweet should have a takeaway or conclusion

CONTENT GUIDELINES:
- Target cadence: ~1 thread per work session + 2-3 standalone tweets
- Be selective - quality over quantity
- Focus on: interesting work, funny moments, accomplishments, relatable developer experiences
- Avoid: mundane tasks, repetitive content, incomplete thoughts

When you find thread-worthy content, group related moments together chronologically.
"#,
        window_start_str,
        window_end_str,
        activity_summary,
        capture_summary
    );

    parts.push(MediaPart::Text { text: prompt });

    let message = MultimodalMessage {
        role: ChatRole::User,
        parts,
        cache_marker: None,
    };

    let mut history = vec![ConversationMessage::Multimodal(message.clone())];

    // Run agent loop
    for _turn in 0..MAX_TURNS {
        println!("[agent] Starting turn {}", _turn + 1);
        if ctx.lock().await.completed {
            break;
        }

        let response = runtime
            .run(
                None,
                Some("You are a social media content curator. Find tweet-worthy moments from screen recordings."),
                Some(history.clone()),
                None,
                None,
                None,
                None,
                None,
                None,
            )
            .await?;

        println!("[agent] Turn {} response: {:?}", _turn + 1, response);

        // Append tool call + result messages back into the running history
        let response_is_tool_array = response
            .as_array()
            .map(|arr| arr.iter().all(|value| runtime.is_tool_call(value)))
            .unwrap_or(false);

        let mut tool_call_values: Vec<serde_json::Value> = Vec::new();
        if runtime.is_tool_call(&response) {
            tool_call_values.push(response.clone());
        } else if response_is_tool_array {
            if let Some(arr) = response.as_array() {
                tool_call_values.extend(arr.iter().cloned());
            }
        }

        for call_value in &tool_call_values {
            match ToolCall::create(call_value.clone()) {
                Ok(CreateResult::Single(tool_call)) => {
                    history.push(ConversationMessage::ToolCall(tool_call.clone()));

                    let execution_result = runtime.execute_tool(call_value).await;
                    let tool_result = match execution_result {
                        Ok(content) => ToolResult::success_with_name(
                            tool_call.tool_use_id.clone(),
                            tool_call.tool_name.clone(),
                            content,
                        )
                        .with_tool_obj(tool_call.args.clone()),
                        Err(err) => ToolResult::error(
                            tool_call.tool_use_id.clone(),
                            format!("Tool execution failed: {}", err),
                        )
                        .with_tool_name(tool_call.tool_name.clone())
                        .with_tool_obj(tool_call.args.clone()),
                    };

                    history.push(ConversationMessage::ToolResult(tool_result));
                }
                Ok(CreateResult::Multiple(_)) => {
                    eprintln!(
                        "[agent] Unexpected nested tool call payload when updating history"
                    );
                }
                Ok(CreateResult::Empty) => {}
                Err(err) => {
                    eprintln!(
                        "[agent] Failed to parse tool call for history reinjection: {}",
                        err
                    );
                }
            }
        }

        if tool_call_values.is_empty() {
            if !response.is_null() {
                let assistant_text = response
                    .as_str()
                    .map(|s| s.to_string())
                    .unwrap_or_else(|| serde_json::to_string(&response).unwrap_or_default());

                if !assistant_text.is_empty() {
                    history.push(ConversationMessage::Chat(ChatMessage::assistant(
                        assistant_text,
                    )));
                }
            }
        }

        // Check if agent is done (marked complete via tool)
        if ctx.lock().await.completed {
            break;
        }

        // If no tool calls in response, agent is done thinking
        // if response.is_string() {
        //     break;
        // }
    }

    Ok(())
}

// Main entry point for the background job

pub async fn run_collateral_job(
    db: PgPool,
    gcs: Storage,
    gemini_client: GoogleGenAIClient,
    user_id: i64,
) -> Result<Vec<TweetCollateral>, Box<dyn std::error::Error + Send + Sync>> {
    // Determine time window
    let now = Utc::now();
    let window_start = get_last_run_time(&db, user_id)
        .await
        .unwrap_or_else(|| now - Duration::hours(4));
    let window_end = now;

    println!(
        "[agent] User {} - processing window {} to {}",
        user_id, window_start, window_end
    );

    // Fetch data
    let captures = fetch_captures_in_window(&db, user_id, window_start, window_end).await?;
    let activities = fetch_activities_in_window(&db, user_id, window_start, window_end).await?;

    if captures.is_empty() {
        println!("[agent] User {} - no captures found in window", user_id);
        // Nothing to process - record run for tracking (don't fail if this errors)
        if let Err(e) = record_run(&db, user_id, window_start, window_end, 0).await {
            eprintln!("[agent] Failed to record empty run: {}", e);
        }
        return Ok(vec![]);
    }

    // Upload media to Gemini
    // - Videos: Use File API (large files, need processing)
    // - Images: Base64 encode inline (smaller, no processing needed)
    let mut uploaded_media: Vec<UploadedMedia> = Vec::new();
    let mut uploaded_file_names: Vec<String> = Vec::new(); // For cleanup

    for capture in &captures {
        // Download from GCS
        let bucket = format!("projects/_/buckets/{}", BUCKET_NAME);

        println!(
            "[agent] User {} - downloading capture {} from GCS: {}",
            user_id, capture.id, capture.gcs_path
        );
        let mut resp = gcs.read_object(&bucket, &capture.gcs_path).send().await?;

        let mut data = Vec::new();
        while let Some(chunk) = resp.next().await {
            data.extend_from_slice(&chunk?);
        }

        println!(
            "[agent] User {} - downloaded capture {} ({} bytes)",
            user_id,
            capture.id,
            data.len()
        );

        if capture.media_type == "video" {
            // Upload video to Gemini File API
            let uploaded = gemini_client
                .upload_file(
                    &data,
                    &capture.content_type,
                    Some(&format!("capture_{}", capture.id)),
                )
                .await?;

            println!(
                "[agent] User {} - uploaded video capture {} to Gemini File API: {} {:#?}",
                user_id, capture.id, uploaded.name, uploaded.state
            );

            if uploaded.state == FileState::Processing {
                let out = gemini_client
                    .wait_for_file_processing(&uploaded.name, Some(120))
                    .await?;
                println!(
                    "[agent] User {} - video capture {} processing complete: {:#?}",
                    user_id, capture.id, out
                );
            }

            uploaded_media.push(UploadedMedia {
                capture_id: capture.id,
                uri: uploaded.uri.clone(),
                mime_type: capture.content_type.clone(),
                is_video: true,
            });
            uploaded_file_names.push(uploaded.name);
        } else {
            // Images: base64 encode for inline embedding
            let b64 = base64::engine::general_purpose::STANDARD.encode(&data);
            uploaded_media.push(UploadedMedia {
                capture_id: capture.id,
                uri: b64,
                mime_type: capture.content_type.clone(),
                is_video: false,
            });
        }
    }

    // Create agent context
    let context = Arc::new(Mutex::new(AgentContext {
        db: db.clone(),
        gcs: gcs.clone(),
        user_id,
        window_start,
        window_end,
        tweets: Vec::new(),
        threads: Vec::new(),
        completed: false,
        next_thread_id: 1,
    }));

    // Run agent - ensure cleanup happens even on error
    let agent_result = run_collateral_agent(context.clone(), captures, activities, uploaded_media).await;

    // Helper to cleanup uploaded files
    async fn cleanup_gemini_files(client: &GoogleGenAIClient, file_names: &[String]) {
        for file_name in file_names {
            if let Err(e) = client.delete_file(file_name).await {
                eprintln!("[agent] Failed to cleanup Gemini file {}: {}", file_name, e);
            }
        }
    }

    // If agent failed, cleanup and return error
    if let Err(e) = agent_result {
        cleanup_gemini_files(&gemini_client, &uploaded_file_names).await;
        return Err(e.into());
    }

    // Get results
    let guard = context.lock().await;
    let tweets = guard.tweets.clone();
    let threads = guard.threads.clone();
    drop(guard); // Release lock before DB operations

    // Save threads and tweets atomically - if any fails, all are rolled back
    if let Err(e) = save_threads_and_tweets(&db, user_id, &threads, &tweets).await {
        cleanup_gemini_files(&gemini_client, &uploaded_file_names).await;
        return Err(e.into());
    }

    // Record run - if this fails, cleanup but don't error (tweets are already saved)
    if let Err(e) = record_run(&db, user_id, window_start, window_end, tweets.len() as i32).await {
        eprintln!("[agent] Failed to record run (tweets already saved): {}", e);
    }

    // Cleanup uploaded video files from Gemini File API
    cleanup_gemini_files(&gemini_client, &uploaded_file_names).await;

    Ok(tweets)
}

/// Background scheduler that runs the agent for idle users
pub async fn start_background_scheduler(
    db: PgPool,
    gcs: Storage,
    gemini_client: GoogleGenAIClient,
    idle_minutes: i64,
    check_interval_secs: u64,
) {
    let mut interval = tokio::time::interval(std::time::Duration::from_secs(check_interval_secs));

    loop {
        interval.tick().await;

        // Find idle users with pending captures
        match find_idle_users_with_pending_captures(&db, idle_minutes).await {
            Ok(user_ids) => {
                for user_id in user_ids {
                    println!("[scheduler] Processing idle user {}", user_id);

                    match run_collateral_job(
                        db.clone(),
                        gcs.clone(),
                        gemini_client.clone(),
                        user_id,
                    )
                    .await
                    {
                        Ok(tweets) => {
                            println!(
                                "[scheduler] User {} - generated {} tweets",
                                user_id,
                                tweets.len()
                            );
                        }
                        Err(e) => {
                            eprintln!("[scheduler] User {} - error: {}", user_id, e);
                        }
                    }
                }
            }
            Err(e) => {
                eprintln!("[scheduler] Error finding idle users: {}", e);
            }
        }
    }
}
