//! Pure identity rules: value types, events, state and decisions. No I/O.

use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use serde_json::Value;
use uuid::Uuid;

pub const INVITATION_TTL: SignedDuration = SignedDuration::from_hours(24 * 7);

#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
pub enum DomainError {
    #[error("invalid email address")]
    InvalidEmail,
    #[error("display name must be 1-100 characters")]
    InvalidDisplayName,
    #[error("passkey name must be 1-64 characters")]
    InvalidPasskeyName,
    #[error("registration requires an invitation")]
    InvitationRequired,
    #[error("invitation has expired")]
    InvitationExpired,
    #[error("invitation has already been used")]
    InvitationAlreadyUsed,
    #[error("email does not match the invitation")]
    InvitationEmailMismatch,
    #[error("passkey is already registered")]
    DuplicatePasskey,
    #[error("unknown passkey")]
    UnknownPasskey,
    #[error("only admins may do this")]
    NotAdmin,
}

/// Normalized (trimmed, lowercase) email address.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct Email(String);

impl Email {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        let email = raw.trim().to_lowercase();
        let well_formed = match email.split_once('@') {
            Some((local, domain)) => {
                !local.is_empty()
                    && !domain.contains('@')
                    && domain.contains('.')
                    && !domain.starts_with('.')
                    && !domain.ends_with('.')
            }
            None => false,
        };
        if well_formed && email.len() <= 254 && !email.contains(char::is_whitespace) {
            Ok(Self(email))
        } else {
            Err(DomainError::InvalidEmail)
        }
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

fn bounded_text(raw: &str, max_chars: usize) -> Option<String> {
    let text = raw.trim();
    (!text.is_empty() && text.chars().count() <= max_chars).then(|| text.to_owned())
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(transparent)]
pub struct DisplayName(String);

impl DisplayName {
    pub fn parse(raw: &str) -> Result<Self, DomainError> {
        bounded_text(raw, 100)
            .map(Self)
            .ok_or(DomainError::InvalidDisplayName)
    }

    pub fn as_str(&self) -> &str {
        &self.0
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Role {
    Admin,
    Member,
}

impl Role {
    pub fn as_str(self) -> &'static str {
        match self {
            Role::Admin => "admin",
            Role::Member => "member",
        }
    }
}

/// A registered WebAuthn credential. `passkey` is the serialized
/// `webauthn_rs::prelude::Passkey`; this crate treats it as opaque.
#[derive(Debug, Clone, PartialEq)]
pub struct Passkey {
    pub credential_id: String,
    pub name: String,
    pub passkey: Value,
}

impl Passkey {
    pub fn new(credential_id: String, name: &str, passkey: Value) -> Result<Self, DomainError> {
        let name = bounded_text(name, 64).ok_or(DomainError::InvalidPasskeyName)?;
        Ok(Self {
            credential_id,
            name,
            passkey,
        })
    }
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum UserEvent {
    UserRegistered {
        user_id: Uuid,
        email: Email,
        display_name: DisplayName,
        role: Role,
        invitation_id: Option<Uuid>,
    },
    PasskeyAdded {
        credential_id: String,
        name: String,
        passkey: Value,
    },
    /// A successful login. Carries the credential with its updated counter.
    PasskeyUsed {
        credential_id: String,
        passkey: Value,
    },
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type")]
pub enum InvitationEvent {
    InvitationCreated {
        invitation_id: Uuid,
        email: Email,
        token_hash: String,
        created_by: Uuid,
        expires_at: Timestamp,
    },
    InvitationAccepted {
        user_id: Uuid,
    },
}

#[derive(Debug, Clone, PartialEq)]
pub struct User {
    pub id: Uuid,
    pub email: Email,
    pub display_name: DisplayName,
    pub role: Role,
    pub passkeys: Vec<Passkey>,
}

impl User {
    /// Folds a user stream into state; `None` for an empty stream.
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a UserEvent>) -> Option<Self> {
        let mut user: Option<Self> = None;
        for event in events {
            match (event, user.as_mut()) {
                (
                    UserEvent::UserRegistered {
                        user_id,
                        email,
                        display_name,
                        role,
                        ..
                    },
                    _,
                ) => {
                    user = Some(Self {
                        id: *user_id,
                        email: email.clone(),
                        display_name: display_name.clone(),
                        role: *role,
                        passkeys: Vec::new(),
                    });
                }
                (
                    UserEvent::PasskeyAdded {
                        credential_id,
                        name,
                        passkey,
                    },
                    Some(user),
                ) => user.passkeys.push(Passkey {
                    credential_id: credential_id.clone(),
                    name: name.clone(),
                    passkey: passkey.clone(),
                }),
                (
                    UserEvent::PasskeyUsed {
                        credential_id,
                        passkey,
                    },
                    Some(user),
                ) => {
                    if let Some(p) = user
                        .passkeys
                        .iter_mut()
                        .find(|p| &p.credential_id == credential_id)
                    {
                        p.passkey = passkey.clone();
                    }
                }
                (_, None) => {}
            }
        }
        user
    }

    fn passkey(&self, credential_id: &str) -> Option<&Passkey> {
        self.passkeys
            .iter()
            .find(|p| p.credential_id == credential_id)
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Invitation {
    pub id: Uuid,
    pub email: Email,
    pub token_hash: String,
    pub created_by: Uuid,
    pub expires_at: Timestamp,
    pub accepted_by: Option<Uuid>,
}

impl Invitation {
    pub fn from_events<'a>(events: impl IntoIterator<Item = &'a InvitationEvent>) -> Option<Self> {
        let mut invitation: Option<Self> = None;
        for event in events {
            match (event, invitation.as_mut()) {
                (
                    InvitationEvent::InvitationCreated {
                        invitation_id,
                        email,
                        token_hash,
                        created_by,
                        expires_at,
                    },
                    _,
                ) => {
                    invitation = Some(Self {
                        id: *invitation_id,
                        email: email.clone(),
                        token_hash: token_hash.clone(),
                        created_by: *created_by,
                        expires_at: *expires_at,
                        accepted_by: None,
                    });
                }
                (InvitationEvent::InvitationAccepted { user_id }, Some(inv)) => {
                    inv.accepted_by = Some(*user_id);
                }
                (_, None) => {}
            }
        }
        invitation
    }
}

/// On what grounds someone is allowed to register.
#[derive(Debug, Clone, Copy)]
pub enum Admission<'a> {
    /// No users exist yet: the first user becomes admin.
    Bootstrap,
    Invited(&'a Invitation),
    Uninvited,
}

#[derive(Debug, Clone)]
pub struct RegisterUser {
    pub user_id: Uuid,
    pub email: Email,
    pub display_name: DisplayName,
    pub passkey: Passkey,
}

/// Events for the new user's stream, plus the invitation event if one was used.
pub fn register_user(
    admission: Admission<'_>,
    cmd: RegisterUser,
    now: Timestamp,
) -> Result<(Vec<UserEvent>, Option<InvitationEvent>), DomainError> {
    let (role, invitation_id) = match admission {
        Admission::Bootstrap => (Role::Admin, None),
        Admission::Uninvited => return Err(DomainError::InvitationRequired),
        Admission::Invited(invitation) => {
            if invitation.accepted_by.is_some() {
                return Err(DomainError::InvitationAlreadyUsed);
            }
            if now >= invitation.expires_at {
                return Err(DomainError::InvitationExpired);
            }
            if invitation.email != cmd.email {
                return Err(DomainError::InvitationEmailMismatch);
            }
            (Role::Member, Some(invitation.id))
        }
    };
    let user_events = vec![
        UserEvent::UserRegistered {
            user_id: cmd.user_id,
            email: cmd.email,
            display_name: cmd.display_name,
            role,
            invitation_id,
        },
        UserEvent::PasskeyAdded {
            credential_id: cmd.passkey.credential_id,
            name: cmd.passkey.name,
            passkey: cmd.passkey.passkey,
        },
    ];
    let accepted = invitation_id.map(|_| InvitationEvent::InvitationAccepted {
        user_id: cmd.user_id,
    });
    Ok((user_events, accepted))
}

pub fn add_passkey(user: &User, passkey: Passkey) -> Result<Vec<UserEvent>, DomainError> {
    if user.passkey(&passkey.credential_id).is_some() {
        return Err(DomainError::DuplicatePasskey);
    }
    Ok(vec![UserEvent::PasskeyAdded {
        credential_id: passkey.credential_id,
        name: passkey.name,
        passkey: passkey.passkey,
    }])
}

pub fn record_passkey_use(
    user: &User,
    credential_id: &str,
    passkey: Value,
) -> Result<Vec<UserEvent>, DomainError> {
    user.passkey(credential_id)
        .ok_or(DomainError::UnknownPasskey)?;
    Ok(vec![UserEvent::PasskeyUsed {
        credential_id: credential_id.to_owned(),
        passkey,
    }])
}

pub fn create_invitation(
    creator: &User,
    invitation_id: Uuid,
    email: Email,
    token_hash: String,
    now: Timestamp,
) -> Result<InvitationEvent, DomainError> {
    if creator.role != Role::Admin {
        return Err(DomainError::NotAdmin);
    }
    Ok(InvitationEvent::InvitationCreated {
        invitation_id,
        email,
        token_hash,
        created_by: creator.id,
        expires_at: now + INVITATION_TTL,
    })
}
