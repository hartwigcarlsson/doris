//! Users, passkeys and invitations, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
mod projections;
mod queries;
mod session;
pub mod token;
mod webauthn;

use domain::{
    Admission, DisplayName, DomainError, Email, Invitation, InvitationEvent, Passkey, RegisterUser,
    User, UserEvent,
};
use doris_eventstore::{Metadata, NewEvent};
use jiff::Timestamp;
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::{
    InvitationSummary, PasskeySummary, bootstrap_required, invitation_email, list_invitations,
    list_passkeys,
};
pub use session::{SESSION_TTL, create_session, end_session, session_user};
pub use webauthn::{Auth, CEREMONY_TTL};

const USER_STREAM: &str = "user-";
const INVITATION_STREAM: &str = "invitation-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("already exists")]
    AlreadyExists,
    #[error("invitation not found")]
    InvitationNotFound,
    #[error("user not found")]
    UserNotFound,
    #[error("ceremony not found")]
    CeremonyNotFound,
    #[error("ceremony expired")]
    CeremonyExpired,
    /// Deliberately vague: never reveals whether the email exists.
    #[error("login failed")]
    LoginFailed,
    #[error(transparent)]
    Webauthn(#[from] webauthn_rs::prelude::WebauthnError),
    #[error(transparent)]
    Store(doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<doris_eventstore::Error> for Error {
    fn from(err: doris_eventstore::Error) -> Self {
        match err {
            doris_eventstore::Error::Db(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Error::AlreadyExists
            }
            other => Error::Store(other),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        doris_eventstore::Error::from(err).into()
    }
}

/// Registers a user with their first passkey. The very first user becomes
/// admin (any invitation token is then ignored); everyone after that needs a
/// valid invitation for the same email.
///
/// The caller chooses `user_id`: the WebAuthn ceremony picks it at
/// `begin_registration`, where it becomes the WebAuthn user handle.
pub async fn register(
    pool: &SqlitePool,
    user_id: Uuid,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    passkey: Passkey,
    now: Timestamp,
) -> Result<User> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let user = register_in(
        &mut tx,
        user_id,
        email,
        display_name,
        invitation_token,
        passkey,
        now,
    )
    .await?;
    tx.commit().await?;
    Ok(user)
}

/// Runs every registration rule without saving anything, so a WebAuthn
/// ceremony can refuse before the authenticator creates a credential.
pub async fn check_registration(
    pool: &SqlitePool,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    now: Timestamp,
) -> Result<()> {
    let placeholder = Passkey::new(String::new(), "placeholder", serde_json::Value::Null)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    register_in(
        &mut tx,
        Uuid::new_v4(),
        email,
        display_name,
        invitation_token,
        placeholder,
        now,
    )
    .await?;
    tx.rollback().await?;
    Ok(())
}

async fn register_in(
    conn: &mut SqliteConnection,
    user_id: Uuid,
    email: &str,
    display_name: &str,
    invitation_token: Option<&str>,
    passkey: Passkey,
    now: Timestamp,
) -> Result<User> {
    let cmd = RegisterUser {
        user_id,
        email: Email::parse(email)?,
        display_name: DisplayName::parse(display_name)?,
        passkey,
    };
    let user_count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM users")
        .fetch_one(&mut *conn)
        .await?;
    let invitation = match invitation_token {
        Some(token) if user_count > 0 => Some(load_invitation_by_token(conn, token).await?),
        _ => None,
    };
    let admission = match (&invitation, user_count) {
        (_, 0) => Admission::Bootstrap,
        (Some((invitation, _)), _) => Admission::Invited(invitation),
        (None, _) => Admission::Uninvited,
    };

    let (user_events, accepted) = domain::register_user(admission, cmd, now)?;
    let actor = Some(user_id);
    commit(conn, &user_stream(user_id), 0, &user_events, actor).await?;
    if let (Some(event), Some((invitation, version))) = (accepted, &invitation) {
        let stream = invitation_stream(invitation.id);
        commit(conn, &stream, *version, &[event], actor).await?;
    }
    Ok(User::from_events(&user_events).expect("registration yields a user"))
}

/// Creates an email-bound invitation. Returns its id and the plaintext token,
/// which is never stored.
pub async fn create_invitation(
    pool: &SqlitePool,
    creator_id: Uuid,
    email: &str,
    now: Timestamp,
) -> Result<(Uuid, String)> {
    let email = Email::parse(email)?;
    let mut tx = doris_eventstore::begin(pool).await?;
    let (creator, _) = load_user(&mut tx, creator_id)
        .await?
        .ok_or(Error::UserNotFound)?;

    let taken: bool = sqlx::query_scalar(
        "SELECT EXISTS (SELECT 1 FROM users WHERE email = ?1)
             OR EXISTS (SELECT 1 FROM invitations
                        WHERE email = ?1 AND accepted_by IS NULL AND expires_at > ?2)",
    )
    .bind(email.as_str())
    .bind(now.as_second())
    .fetch_one(&mut *tx)
    .await?;
    if taken {
        return Err(Error::AlreadyExists);
    }

    let id = Uuid::new_v4();
    let token = token::new_token();
    let event = domain::create_invitation(&creator, id, email, token::hash_token(&token), now)?;
    commit(
        &mut tx,
        &invitation_stream(id),
        0,
        &[event],
        Some(creator_id),
    )
    .await?;
    tx.commit().await?;
    Ok((id, token))
}

pub async fn add_passkey(pool: &SqlitePool, user_id: Uuid, passkey: Passkey) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (user, version) = load_user(&mut tx, user_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let events = domain::add_passkey(&user, passkey)?;
    commit(
        &mut tx,
        &user_stream(user_id),
        version,
        &events,
        Some(user_id),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// Records a successful login with the credential's updated state.
pub async fn record_passkey_use(
    pool: &SqlitePool,
    user_id: Uuid,
    credential_id: &str,
    passkey: serde_json::Value,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (user, version) = load_user(&mut tx, user_id)
        .await?
        .ok_or(Error::UserNotFound)?;
    let events = domain::record_passkey_use(&user, credential_id, passkey)?;
    commit(
        &mut tx,
        &user_stream(user_id),
        version,
        &events,
        Some(user_id),
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

pub async fn get_user(pool: &SqlitePool, user_id: Uuid) -> Result<Option<User>> {
    let mut conn = pool.acquire().await?;
    Ok(load_user(&mut conn, user_id).await?.map(|(user, _)| user))
}

/// Looks a user up by email (case-insensitive, via normalization).
pub async fn find_user_by_email(pool: &SqlitePool, email: &str) -> Result<Option<User>> {
    let Ok(email) = Email::parse(email) else {
        return Ok(None);
    };
    let mut conn = pool.acquire().await?;
    let user_id: Option<String> = sqlx::query_scalar("SELECT user_id FROM users WHERE email = ?")
        .bind(email.as_str())
        .fetch_optional(&mut *conn)
        .await?;
    match user_id {
        Some(id) => Ok(load_user(&mut conn, id.parse().expect("user_id is a uuid"))
            .await?
            .map(|(user, _)| user)),
        None => Ok(None),
    }
}

fn user_stream(id: Uuid) -> String {
    format!("{USER_STREAM}{id}")
}

fn invitation_stream(id: Uuid) -> String {
    format!("{INVITATION_STREAM}{id}")
}

async fn load_stream<T: DeserializeOwned>(
    conn: &mut SqliteConnection,
    stream: &str,
) -> Result<(Vec<T>, i64)> {
    let recorded = doris_eventstore::load(conn, stream).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode())
        .collect::<Result<_, _>>()?;
    Ok((events, version))
}

async fn load_user(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<(User, i64)>> {
    let (events, version) = load_stream::<UserEvent>(conn, &user_stream(id)).await?;
    Ok(User::from_events(&events).map(|user| (user, version)))
}

async fn load_invitation_by_token(
    conn: &mut SqliteConnection,
    token: &str,
) -> Result<(Invitation, i64)> {
    let id: String =
        sqlx::query_scalar("SELECT invitation_id FROM invitations WHERE token_hash = ?")
            .bind(token::hash_token(token))
            .fetch_optional(&mut *conn)
            .await?
            .ok_or(Error::InvitationNotFound)?;
    let id: Uuid = id.parse().expect("invitation_id is a uuid");
    let (events, version) = load_stream::<InvitationEvent>(conn, &invitation_stream(id)).await?;
    let invitation = Invitation::from_events(&events).ok_or(Error::InvitationNotFound)?;
    Ok((invitation, version))
}

/// Appends events and updates projections within the caller's transaction.
async fn commit<T: Serialize>(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[T],
    actor: Option<Uuid>,
) -> Result<()> {
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: actor.map(|id| id.to_string()),
    };
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
