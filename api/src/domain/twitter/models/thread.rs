//! Thread model definitions

use chrono::{DateTime, Utc};
use serde::{Deserialize, Serialize};
use sqlx::encode::IsNull;
use sqlx::error::BoxDynError;
use sqlx::postgres::{PgArgumentBuffer, PgTypeInfo, PgValueRef};
use sqlx::{Decode, Encode, Postgres, Type};

use super::tweet::Tweet;

/// Thread status enum
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "snake_case")]
pub enum ThreadStatus {
    Draft,
    Posting,
    Posted,
    PartialFailed,
}

impl ThreadStatus {
    pub fn as_str(&self) -> &'static str {
        match self {
            ThreadStatus::Draft => "draft",
            ThreadStatus::Posting => "posting",
            ThreadStatus::Posted => "posted",
            ThreadStatus::PartialFailed => "partial_failed",
        }
    }

    pub fn from_str(s: &str) -> Self {
        match s {
            "draft" => ThreadStatus::Draft,
            "posting" => ThreadStatus::Posting,
            "posted" => ThreadStatus::Posted,
            "partial_failed" => ThreadStatus::PartialFailed,
            _ => ThreadStatus::Draft,
        }
    }
}

// sqlx Type/Decode/Encode for ThreadStatus to enable FromRow on Thread
impl Type<Postgres> for ThreadStatus {
    fn type_info() -> PgTypeInfo {
        <String as Type<Postgres>>::type_info()
    }

    fn compatible(ty: &PgTypeInfo) -> bool {
        <String as Type<Postgres>>::compatible(ty)
    }
}

impl<'r> Decode<'r, Postgres> for ThreadStatus {
    fn decode(value: PgValueRef<'r>) -> Result<Self, BoxDynError> {
        let s = <String as Decode<Postgres>>::decode(value)?;
        Ok(ThreadStatus::from_str(&s))
    }
}

impl Encode<'_, Postgres> for ThreadStatus {
    fn encode_by_ref(&self, buf: &mut PgArgumentBuffer) -> Result<IsNull, BoxDynError> {
        <String as Encode<Postgres>>::encode_by_ref(&self.as_str().to_owned(), buf)
    }
}

/// A tweet thread container
#[derive(Debug, Clone, sqlx::FromRow)]
pub struct Thread {
    pub id: i64,
    #[allow(dead_code)] // Fetched from DB but intentionally not exposed in API responses
    pub user_id: i64,
    pub title: Option<String>,
    pub status: ThreadStatus,
    pub created_at: DateTime<Utc>,
    pub posted_at: Option<DateTime<Utc>>,
    pub first_tweet_id: Option<String>,
}

/// Thread with its tweets (domain composition)
#[derive(Debug, Clone)]
pub struct ThreadWithTweets {
    pub thread: Thread,
    pub tweets: Vec<Tweet>,
}
