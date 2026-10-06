//! API tokens: event-sourced grants per company, looked up by hash on
//! every call. When a token was last used is operational state.

use crate::domain::{self, ApiToken, ApiTokenEvent, Grant, NewApiToken, User};
use crate::{API_TOKEN_STREAM, Error, Result, commit, get_user, load_stream, load_user, token};
use jiff::Timestamp;
use sqlx::SqlitePool;
use uuid::Uuid;

/// Every API token starts with this, so it is recognisable in a config file.
pub const API_TOKEN_PREFIX: &str = "doris_";
/// Last use is written at most this often (seconds), so reads rarely write.
const USAGE_INTERVAL: i64 = 3600;

#[derive(Debug, Clone, PartialEq)]
pub struct ApiTokenSummary {
    pub id: Uuid,
    pub name: String,
    pub grants: Vec<Grant>,
    pub created_at: String,
    pub expires_at: Timestamp,
    pub last_used_at: Option<Timestamp>,
    pub revoked_at: Option<String>,
}

/// What a live token may reach.
#[derive(Debug, Clone, PartialEq)]
pub struct TokenAccess {
    pub token_id: Uuid,
    pub grants: Vec<Grant>,
}

fn stream(id: Uuid) -> String {
    format!("{API_TOKEN_STREAM}{id}")
}

fn timestamp(seconds: i64) -> Timestamp {
    Timestamp::from_second(seconds).expect("stored timestamps are valid")
}

/// Creates a token. Returns its id and the plaintext, which is never stored.
pub async fn create_api_token(
    pool: &SqlitePool,
    owner_id: Uuid,
    name: &str,
    expires_at: Timestamp,
    grants: Vec<Grant>,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (owner, _) = load_user(&mut tx, owner_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let id = Uuid::new_v4();
    let secret = format!("{API_TOKEN_PREFIX}{}", token::new_token());
    let cmd = NewApiToken {
        token_id: id,
        name: name.to_owned(),
        expires_at,
        grants,
    };
    let event = domain::create_api_token(&owner, cmd, token::hash_token(&secret), now)?;
    commit(&mut tx, &stream(id), 0, &[event], Some(owner_id)).await?;
    tx.commit().await?;
    Ok((id, secret))
}

pub async fn revoke_api_token(pool: &SqlitePool, actor_id: Uuid, token_id: Uuid) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (actor, _) = load_user(&mut tx, actor_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let (events, version) = load_stream::<ApiTokenEvent>(&mut tx, &stream(token_id)).await?;
    let token = ApiToken::from_events(&events).ok_or(Error::ApiTokenNotFound)?;
    let events = domain::revoke_api_token(&token, &actor)?;
    commit(&mut tx, &stream(token_id), version, &events, Some(actor_id)).await?;
    tx.commit().await?;
    Ok(())
}

/// A user's tokens, newest first, revoked and expired ones too.
pub async fn list_api_tokens(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<ApiTokenSummary>> {
    type Row = (
        String,
        String,
        String,
        String,
        i64,
        Option<String>,
        Option<i64>,
    );
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT t.token_id, t.name, t.grants, t.created_at, t.expires_at, t.revoked_at,
                u.last_used_at
         FROM api_tokens t LEFT JOIN api_token_usage u ON u.token_id = t.token_id
         WHERE t.user_id = ? ORDER BY t.created_at DESC, t.rowid DESC",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(
            |(id, name, grants, created_at, expires_at, revoked_at, last_used_at)| {
                Ok(ApiTokenSummary {
                    id: id.parse().expect("token_id is a uuid"),
                    name,
                    grants: serde_json::from_str(&grants)?,
                    created_at,
                    expires_at: timestamp(expires_at),
                    last_used_at: last_used_at.map(timestamp),
                    revoked_at,
                })
            },
        )
        .collect()
}

/// The owner of a live token and what it may reach; `None` for an unknown,
/// expired or revoked one.
pub async fn token_user(
    pool: &SqlitePool,
    secret: &str,
    now: Timestamp,
) -> Result<Option<(User, TokenAccess)>> {
    if !secret.starts_with(API_TOKEN_PREFIX) {
        return Ok(None);
    }
    let row: Option<(String, String, String)> = sqlx::query_as(
        "SELECT token_id, user_id, grants FROM api_tokens
         WHERE token_hash = ? AND revoked_at IS NULL AND expires_at > ?",
    )
    .bind(token::hash_token(secret))
    .bind(now.as_second())
    .fetch_optional(pool)
    .await?;
    let Some((token_id, user_id, grants)) = row else {
        return Ok(None);
    };
    let Some(user) = get_user(pool, user_id.parse().expect("user_id is a uuid")).await? else {
        return Ok(None);
    };
    let access = TokenAccess {
        token_id: token_id.parse().expect("token_id is a uuid"),
        grants: serde_json::from_str(&grants)?,
    };
    Ok(Some((user, access)))
}

/// Notes that a token was used. Reads first, so most calls write nothing.
pub async fn touch_api_token(pool: &SqlitePool, token_id: Uuid, now: Timestamp) -> Result<()> {
    let last: Option<i64> =
        sqlx::query_scalar("SELECT last_used_at FROM api_token_usage WHERE token_id = ?")
            .bind(token_id.to_string())
            .fetch_optional(pool)
            .await?;
    if last.is_some_and(|at| now.as_second() - at < USAGE_INTERVAL) {
        return Ok(());
    }
    sqlx::query(
        "INSERT INTO api_token_usage (token_id, last_used_at) VALUES (?, ?)
         ON CONFLICT (token_id) DO UPDATE SET last_used_at = excluded.last_used_at",
    )
    .bind(token_id.to_string())
    .bind(now.as_second())
    .execute(pool)
    .await?;
    Ok(())
}
