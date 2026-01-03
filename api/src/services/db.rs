//! Database transaction utilities
//!
//! This module provides utilities for database transaction management.
//! The primary pattern is using sqlx's generic Executor trait, which allows
//! domain functions to accept both `&PgPool` and `&mut PgConnection` (transactions).
//!
//! # Usage Pattern
//!
//! Domain functions should use the generic executor pattern:
//!
//! ```ignore
//! use sqlx::{Executor, Postgres};
//!
//! pub async fn my_query<'e, E>(executor: E, id: i64) -> Result<MyType, sqlx::Error>
//! where
//!     E: Executor<'e, Database = Postgres>,
//! {
//!     sqlx::query_as("SELECT * FROM my_table WHERE id = $1")
//!         .bind(id)
//!         .fetch_one(executor)
//!         .await
//! }
//! ```
//!
//! This function can then be called with either:
//! - `my_query(&pool, id)` - uses connection from pool
//! - `my_query(&mut *tx, id)` - uses transaction
//!
//! # Transaction Management
//!
//! Routes manage transaction boundaries:
//!
//! ```ignore
//! async fn my_handler(State(state): State<Arc<AppState>>) -> Result<Json<Response>, StatusCode> {
//!     let mut tx = state.db.begin().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
//!
//!     domain::do_something(&mut *tx, ...).await?;
//!     domain::do_another_thing(&mut *tx, ...).await?;
//!
//!     tx.commit().await.map_err(|_| StatusCode::INTERNAL_SERVER_ERROR)?;
//!     Ok(Json(response))
//! }
//! ```

// Re-export commonly used types for convenience
// These are currently used via direct sqlx:: imports in domain modules,
// but are available here for future use if needed.
#[allow(unused_imports)]
pub use sqlx::{Executor, Postgres};
