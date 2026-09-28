//! Read-only views over the identity projections, for listing in the UI.

use crate::domain::Email;
use crate::{Result, token};
use jiff::Timestamp;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct PasskeySummary {
    pub credential_id: String,
    pub name: String,
    pub added_at: String,
    pub last_used_at: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct InvitationSummary {
    pub id: Uuid,
    pub email: String,
    pub expires_at: Timestamp,
    pub accepted: bool,
}

/// True until the first user has registered.
pub async fn bootstrap_required(pool: &SqlitePool) -> Result<bool> {
    Ok(
        sqlx::query_scalar("SELECT NOT EXISTS (SELECT 1 FROM users)")
            .fetch_one(pool)
            .await?,
    )
}

/// The email an invitation is for, if the token belongs to an invitation
/// that is still usable (not accepted, not expired).
pub async fn invitation_email(
    pool: &SqlitePool,
    token: &str,
    now: Timestamp,
) -> Result<Option<Email>> {
    let email: Option<String> = sqlx::query_scalar(
        "SELECT email FROM invitations
         WHERE token_hash = ? AND accepted_by IS NULL AND expires_at > ?",
    )
    .bind(token::hash_token(token))
    .bind(now.as_second())
    .fetch_optional(pool)
    .await?;
    Ok(email.map(|e| Email::parse(&e).expect("stored emails are normalized")))
}

/// All invitations, the ones expiring last first.
pub async fn list_invitations(pool: &SqlitePool) -> Result<Vec<InvitationSummary>> {
    let rows: Vec<(String, String, i64, bool)> = sqlx::query_as(
        "SELECT invitation_id, email, expires_at, accepted_by IS NOT NULL
         FROM invitations ORDER BY expires_at DESC, invitation_id",
    )
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, email, expires_at, accepted)| InvitationSummary {
            id: id.parse().expect("invitation_id is a uuid"),
            email,
            expires_at: Timestamp::from_second(expires_at).expect("stored timestamps are valid"),
            accepted,
        })
        .collect())
}

/// A user's passkeys, oldest first.
pub async fn list_passkeys(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<PasskeySummary>> {
    let rows: Vec<(String, String, String, Option<String>)> = sqlx::query_as(
        "SELECT credential_id, name, added_at, last_used_at
         FROM passkeys WHERE user_id = ? ORDER BY added_at, credential_id",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(
            |(credential_id, name, added_at, last_used_at)| PasskeySummary {
                credential_id,
                name,
                added_at,
                last_used_at,
            },
        )
        .collect())
}
