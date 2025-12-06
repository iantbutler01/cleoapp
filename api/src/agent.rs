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

const BUCKET_NAME: &str = "cleo_multimedia_data";
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

// Collateral output types

#[derive(Debug, Clone, Serialize)]
pub struct TweetCollateral {
    pub text: String,
    pub video_clip: Option<VideoClip>,
    pub image_capture_ids: Vec<i64>,
    pub rationale: String,
    pub created_at: DateTime<Utc>,
}

#[derive(Debug, Clone, Serialize)]
pub struct VideoClip {
    pub source_capture_id: i64,
    pub start_timestamp: String,
    pub duration_secs: u32,
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
    pub completed: bool,
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
        "#,
    )
    .bind(user_id)
    .bind(start)
    .bind(end)
    .fetch_all(db)
    .await
}

pub async fn fetch_activities_in_window(
    db: &PgPool,
    _user_id: i64,
    start: DateTime<Utc>,
    end: DateTime<Utc>,
) -> Result<Vec<ActivityRecord>, sqlx::Error> {
    // Activities don't have user_id directly, but we can join through interval
    // For now, fetch all in window - adjust based on your schema
    sqlx::query_as::<_, ActivityRecord>(
        r#"
        SELECT id, timestamp, event_type, application, "window"
        FROM activities
        WHERE timestamp >= $1 AND timestamp < $2
        ORDER BY timestamp ASC
        "#,
    )
    .bind(start)
    .bind(end)
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

pub async fn save_tweet(
    db: &PgPool,
    user_id: i64,
    tweet: &TweetCollateral,
) -> Result<i64, sqlx::Error> {
    let video_clip_json = tweet
        .video_clip
        .as_ref()
        .map(|c| serde_json::to_value(c).unwrap());
    let image_ids: Vec<i64> = tweet.image_capture_ids.clone();

    let row: (i64,) = sqlx::query_as(
        r#"
        INSERT INTO tweet_collateral (user_id, text, video_clip, image_capture_ids, rationale, created_at)
        VALUES ($1, $2, $3, $4, $5, $6)
        RETURNING id
        "#,
    )
    .bind(user_id)
    .bind(&tweet.text)
    .bind(video_clip_json)
    .bind(&image_ids)
    .bind(&tweet.rationale)
    .bind(tweet.created_at)
    .fetch_one(db)
    .await?;

    Ok(row.0)
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

#[agentic(model = "gemini:gemini-2.0-flash")]
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
                let media = media_for_tool;
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
2. Find interesting, funny, or notable moments that would make good tweets
3. Use WriteTweet to create tweet content with specific timestamps
4. When done reviewing, use MarkComplete

Focus on:
- Interesting work being done
- Funny moments or reactions
- Impressive accomplishments
- Relatable developer moments

Be selective - only create tweets for genuinely interesting content. Quality over quantity.
"#,
        // TODO: BAD. Change this to not require locking the context here.
        { ctx.lock().await.window_start.format("%Y-%m-%d %H:%M") },
        { ctx.lock().await.window_end.format("%Y-%m-%d %H:%M") },
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
        // Nothing to process
        record_run(&db, user_id, window_start, window_end, 0).await?;
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
        completed: false,
    }));

    // Run agent
    run_collateral_agent(context.clone(), captures, activities, uploaded_media).await?;

    // Get results
    let guard = context.lock().await;
    let tweets = guard.tweets.clone();

    // Save tweets to DB
    for tweet in &tweets {
        save_tweet(&db, user_id, tweet).await?;
    }

    // Record run
    record_run(&db, user_id, window_start, window_end, tweets.len() as i32).await?;

    // Cleanup uploaded video files from Gemini File API
    for file_name in uploaded_file_names {
        let _ = gemini_client.delete_file(&file_name).await;
    }

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
