use serde::{Deserialize, Serialize};
use sqlx::PgPool;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscriptionKeys {
    pub p256dh: String,
    pub auth: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PushSubscriptionData {
    pub endpoint: String,
    pub keys: PushSubscriptionKeys,
}

#[derive(sqlx::FromRow)]
struct PushSubscriptionRow {
    endpoint: String,
    p256dh: String,
    auth: String,
}

pub async fn upsert_user_push_subscription(
    db: &PgPool,
    user_id: i64,
    payload: &PushSubscriptionData,
    user_agent: Option<&str>,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        INSERT INTO user_push_subscriptions (user_id, endpoint, p256dh, auth, user_agent)
        VALUES ($1, $2, $3, $4, $5)
        ON CONFLICT (user_id, endpoint)
        DO UPDATE SET
            p256dh = EXCLUDED.p256dh,
            auth = EXCLUDED.auth,
            user_agent = EXCLUDED.user_agent,
            updated_at = NOW()
        "#,
    )
    .bind(user_id)
    .bind(&payload.endpoint)
    .bind(&payload.keys.p256dh)
    .bind(&payload.keys.auth)
    .bind(user_agent)
    .execute(db)
    .await?;

    Ok(())
}

pub async fn list_user_push_subscriptions(
    db: &PgPool,
    user_id: i64,
) -> Result<Vec<PushSubscriptionData>, sqlx::Error> {
    let rows = sqlx::query_as::<_, PushSubscriptionRow>(
        r#"
        SELECT endpoint, p256dh, auth
        FROM user_push_subscriptions
        WHERE user_id = $1
        ORDER BY updated_at DESC, id DESC
        "#,
    )
    .bind(user_id)
    .fetch_all(db)
    .await?;

    Ok(rows
        .into_iter()
        .map(|row| PushSubscriptionData {
            endpoint: row.endpoint,
            keys: PushSubscriptionKeys {
                p256dh: row.p256dh,
                auth: row.auth,
            },
        })
        .collect())
}

pub async fn delete_user_push_subscription(
    db: &PgPool,
    user_id: i64,
    endpoint: &str,
) -> Result<(), sqlx::Error> {
    sqlx::query(
        r#"
        DELETE FROM user_push_subscriptions
        WHERE user_id = $1 AND endpoint = $2
        "#,
    )
    .bind(user_id)
    .bind(endpoint)
    .execute(db)
    .await?;

    Ok(())
}
