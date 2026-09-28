//! Login sessions: opaque bearer tokens, stored only as hashes. Sessions are
//! operational state, not events.

use crate::domain::User;
use crate::{Result, get_user, token};
use jiff::{SignedDuration, Timestamp};
use sqlx::SqlitePool;
use uuid::Uuid;

pub const SESSION_TTL: SignedDuration = SignedDuration::from_hours(24 * 30);

/// Starts a session and returns its plaintext token, which is never stored.
/// Also purges expired sessions.
pub async fn create_session(pool: &SqlitePool, user_id: Uuid, now: Timestamp) -> Result<String> {
    let token = token::new_token();
    let mut tx = doris_eventstore::begin(pool).await?;
    sqlx::query("DELETE FROM sessions WHERE expires_at <= ?")
        .bind(now.as_second())
        .execute(&mut *tx)
        .await?;
    sqlx::query("INSERT INTO sessions (token_hash, user_id, expires_at) VALUES (?, ?, ?)")
        .bind(token::hash_token(&token))
        .bind(user_id.to_string())
        .bind((now + SESSION_TTL).as_second())
        .execute(&mut *tx)
        .await?;
    tx.commit().await?;
    Ok(token)
}

/// The user behind a live session; `None` for unknown or expired tokens.
pub async fn session_user(pool: &SqlitePool, token: &str, now: Timestamp) -> Result<Option<User>> {
    let user_id: Option<String> =
        sqlx::query_scalar("SELECT user_id FROM sessions WHERE token_hash = ? AND expires_at > ?")
            .bind(token::hash_token(token))
            .bind(now.as_second())
            .fetch_optional(pool)
            .await?;
    match user_id {
        Some(id) => get_user(pool, id.parse().expect("user_id is a uuid")).await,
        None => Ok(None),
    }
}

pub async fn end_session(pool: &SqlitePool, token: &str) -> Result<()> {
    sqlx::query("DELETE FROM sessions WHERE token_hash = ?")
        .bind(token::hash_token(token))
        .execute(pool)
        .await?;
    Ok(())
}
