//! Read models for users, passkeys and invitations. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::{ApiTokenEvent, InvitationEvent, UserEvent};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(user_id) = event.stream_id.strip_prefix(crate::USER_STREAM) {
        apply_user(conn, user_id, event).await
    } else if let Some(invitation_id) = event.stream_id.strip_prefix(crate::INVITATION_STREAM) {
        apply_invitation(conn, invitation_id, event).await
    } else if let Some(token_id) = event.stream_id.strip_prefix(crate::API_TOKEN_STREAM) {
        apply_api_token(conn, token_id, event).await
    } else {
        Ok(())
    }
}

async fn apply_user(
    conn: &mut SqliteConnection,
    user_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    let at = &event.recorded_at;
    match event.decode::<UserEvent>()? {
        UserEvent::UserRegistered {
            email,
            display_name,
            role,
            ..
        } => {
            sqlx::query(
                "INSERT INTO users (user_id, email, display_name, role, registered_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(user_id)
            .bind(email.as_str())
            .bind(display_name.as_str())
            .bind(role.as_str())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        UserEvent::PasskeyAdded {
            credential_id,
            name,
            passkey,
        } => {
            sqlx::query(
                "INSERT INTO passkeys (credential_id, user_id, name, passkey, added_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(credential_id)
            .bind(user_id)
            .bind(name)
            .bind(passkey.to_string())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        UserEvent::PasskeyUsed {
            credential_id,
            passkey,
        } => {
            sqlx::query(
                "UPDATE passkeys SET passkey = ?, last_used_at = ? WHERE credential_id = ?",
            )
            .bind(passkey.to_string())
            .bind(at)
            .bind(credential_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

async fn apply_invitation(
    conn: &mut SqliteConnection,
    invitation_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode::<InvitationEvent>()? {
        InvitationEvent::InvitationCreated {
            email,
            token_hash,
            created_by,
            expires_at,
            ..
        } => {
            sqlx::query(
                "INSERT INTO invitations (invitation_id, email, token_hash, created_by, expires_at)
                 VALUES (?, ?, ?, ?, ?)",
            )
            .bind(invitation_id)
            .bind(email.as_str())
            .bind(token_hash)
            .bind(created_by.to_string())
            .bind(expires_at.as_second())
            .execute(&mut *conn)
            .await?;
        }
        InvitationEvent::InvitationAccepted { user_id } => {
            sqlx::query("UPDATE invitations SET accepted_by = ? WHERE invitation_id = ?")
                .bind(user_id.to_string())
                .bind(invitation_id)
                .execute(&mut *conn)
                .await?;
        }
    }
    Ok(())
}

async fn apply_api_token(
    conn: &mut SqliteConnection,
    token_id: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode::<ApiTokenEvent>()? {
        ApiTokenEvent::ApiTokenCreated {
            user_id,
            name,
            token_hash,
            expires_at,
            grants,
            ..
        } => {
            sqlx::query(
                "INSERT INTO api_tokens
                     (token_id, user_id, name, token_hash, grants, created_at, expires_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(token_id)
            .bind(user_id.to_string())
            .bind(name)
            .bind(token_hash)
            .bind(serde_json::to_string(&grants)?)
            .bind(&event.recorded_at)
            .bind(expires_at.as_second())
            .execute(&mut *conn)
            .await?;
        }
        ApiTokenEvent::ApiTokenRevoked { .. } => {
            sqlx::query("UPDATE api_tokens SET revoked_at = ? WHERE token_id = ?")
                .bind(&event.recorded_at)
                .bind(token_id)
                .execute(&mut *conn)
                .await?;
        }
        ApiTokenEvent::ApiTokenChanged {
            name,
            expires_at,
            grants,
        } => {
            sqlx::query(
                "UPDATE api_tokens SET name = ?, expires_at = ?, grants = ? WHERE token_id = ?",
            )
            .bind(name)
            .bind(expires_at.as_second())
            .bind(serde_json::to_string(&grants)?)
            .bind(token_id)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// Empties all identity projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in [
        "DELETE FROM api_tokens",
        "DELETE FROM passkeys",
        "DELETE FROM invitations",
        "DELETE FROM users",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
