//! Activities domain - DB queries for activity tracking
//!
//! All functions use the generic Executor pattern, allowing them to work with
//! both `&PgPool` (for standalone queries) and `&mut PgConnection` (for transactions).

use chrono::{DateTime, Utc};
use sqlx::{Executor, Postgres};

/// Insert an activity record
pub async fn insert_activity<'e, E>(
    executor: E,
    user_id: i64,
    timestamp: DateTime<Utc>,
    interval_id: i64,
    event_type: &str,
    application: Option<&str>,
    window: Option<&str>,
) -> Result<(), sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query(
        r#"
        INSERT INTO activities (user_id, timestamp, interval_id, event_type, application, "window")
        VALUES ($1, $2, $3, $4, $5, $6)
        "#,
    )
    .bind(user_id)
    .bind(timestamp)
    .bind(interval_id)
    .bind(event_type)
    .bind(application)
    .bind(window)
    .execute(executor)
    .await?;

    Ok(())
}
