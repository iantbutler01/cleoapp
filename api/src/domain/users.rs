//! User domain - DB queries for users
//!
//! All functions use the generic Executor pattern, allowing them to work with
//! both `&PgPool` (for standalone queries) and `&mut PgConnection` (for transactions).

use sqlx::{Executor, Postgres};

#[derive(Debug, sqlx::FromRow)]
pub struct UserBasicInfo {
    pub twitter_username: String,
}

/// Get basic user info by ID
pub async fn get_user_by_id<'e, E>(
    executor: E,
    user_id: i64,
) -> Result<Option<UserBasicInfo>, sqlx::Error>
where
    E: Executor<'e, Database = Postgres>,
{
    sqlx::query_as(
        "SELECT twitter_username FROM users WHERE id = $1"
    )
    .bind(user_id)
    .fetch_optional(executor)
    .await
}
