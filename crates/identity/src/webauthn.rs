//! WebAuthn ceremonies: registering with a passkey, adding passkeys and
//! logging in. Ceremony state is kept server-side, is single use and expires
//! after [`CEREMONY_TTL`].

use crate::domain::{DisplayName, Email, Passkey, User};
use crate::{
    Error, Result, add_passkey, check_registration, create_session, find_user_by_email, get_user,
    record_passkey_use, register,
};
use base64::Engine;
use base64::engine::general_purpose::URL_SAFE_NO_PAD;
use jiff::{SignedDuration, Timestamp};
use serde::{Deserialize, Serialize};
use sqlx::SqlitePool;
use uuid::Uuid;
use webauthn_rs::fake::{FakePasskeyDistribution, WebauthnFakeCredentialGenerator};
use webauthn_rs::prelude::{
    Base64UrlSafeData, CreationChallengeResponse, CredentialID, PasskeyAuthentication,
    PasskeyRegistration, PublicKeyCredential, RegisterPublicKeyCredential,
    RequestChallengeResponse, Url, Webauthn, WebauthnBuilder,
};
use webauthn_rs_proto::{
    AllowCredentials, PublicKeyCredentialRequestOptions, UserVerificationPolicy,
};

pub const CEREMONY_TTL: SignedDuration = SignedDuration::from_mins(5);

/// Browser timeout webauthn-rs uses for real challenges; fakes must match.
const CHALLENGE_TIMEOUT_MS: u32 = 300_000;

#[derive(Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
enum Ceremony {
    Registration {
        user_id: Uuid,
        email: String,
        display_name: String,
        passkey_name: String,
        state: PasskeyRegistration,
    },
    AddPasskey {
        user_id: Uuid,
        passkey_name: String,
        state: PasskeyRegistration,
    },
    /// `None` fields mark a fake login for an unknown email.
    Login {
        user_id: Option<Uuid>,
        state: Option<PasskeyAuthentication>,
    },
}

pub struct Auth {
    pool: SqlitePool,
    webauthn: Webauthn,
    rp_id: String,
    fake_credential_generator: WebauthnFakeCredentialGenerator<FakePasskeyDistribution>,
}

impl Auth {
    pub async fn new(pool: SqlitePool, rp_id: &str, rp_origin: &Url) -> Result<Self> {
        let webauthn = WebauthnBuilder::new(rp_id, rp_origin)?
            .rp_name("Doris")
            .build()?;
        let key = fake_credential_key(&pool).await?;
        let fake_credential_generator =
            WebauthnFakeCredentialGenerator::<FakePasskeyDistribution>::new(&key)?;
        Ok(Self {
            pool,
            webauthn,
            rp_id: rp_id.to_owned(),
            fake_credential_generator,
        })
    }

    /// Checks every registration rule, then asks the browser to create a
    /// passkey. Nothing is saved until [`Auth::finish_registration`].
    pub async fn begin_registration(
        &self,
        email: &str,
        display_name: &str,
        invitation_token: Option<&str>,
        passkey_name: &str,
        now: Timestamp,
    ) -> Result<(Uuid, CreationChallengeResponse)> {
        let email = Email::parse(email)?;
        let display_name = DisplayName::parse(display_name)?;
        let passkey_name = Passkey::validate_name(passkey_name)?;
        check_registration(
            &self.pool,
            email.as_str(),
            display_name.as_str(),
            invitation_token,
            now,
        )
        .await?;

        let user_id = Uuid::new_v4();
        let (options, state) = self.webauthn.start_passkey_registration(
            user_id,
            email.as_str(),
            display_name.as_str(),
            None,
        )?;
        let ceremony = Ceremony::Registration {
            user_id,
            email: email.as_str().to_owned(),
            display_name: display_name.as_str().to_owned(),
            passkey_name,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the new credential, registers the user and starts a session.
    /// The invitation token is sent again rather than kept server-side.
    pub async fn finish_registration(
        &self,
        ceremony_id: Uuid,
        invitation_token: Option<&str>,
        credential: &RegisterPublicKeyCredential,
        now: Timestamp,
    ) -> Result<(User, String)> {
        let Ceremony::Registration {
            user_id,
            email,
            display_name,
            passkey_name,
            state,
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::CeremonyNotFound);
        };
        let passkey = self
            .webauthn
            .finish_passkey_registration(credential, &state)?;
        let passkey = Passkey::new(
            credential_id(passkey.cred_id()),
            &passkey_name,
            serde_json::to_value(&passkey)?,
        )?;
        let user = register(
            &self.pool,
            user_id,
            &email,
            &display_name,
            invitation_token,
            passkey,
            now,
        )
        .await?;
        let session = create_session(&self.pool, user.id, now).await?;
        Ok((user, session))
    }

    /// Asks the browser to create another passkey for a logged-in user.
    pub async fn begin_add_passkey(
        &self,
        user_id: Uuid,
        passkey_name: &str,
        now: Timestamp,
    ) -> Result<(Uuid, CreationChallengeResponse)> {
        let passkey_name = Passkey::validate_name(passkey_name)?;
        let user = get_user(&self.pool, user_id)
            .await?
            .ok_or(Error::UserNotFound)?;
        let existing = user
            .passkeys
            .iter()
            .map(|p| Ok(webauthn_passkey(p)?.cred_id().clone()))
            .collect::<Result<Vec<_>>>()?;
        let (options, state) = self.webauthn.start_passkey_registration(
            user.id,
            user.email.as_str(),
            user.display_name.as_str(),
            Some(existing),
        )?;
        let ceremony = Ceremony::AddPasskey {
            user_id,
            passkey_name,
            state,
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    pub async fn finish_add_passkey(
        &self,
        user_id: Uuid,
        ceremony_id: Uuid,
        credential: &RegisterPublicKeyCredential,
        now: Timestamp,
    ) -> Result<()> {
        let Ceremony::AddPasskey {
            user_id: owner,
            passkey_name,
            state,
        } = self.take(ceremony_id, now).await?
        else {
            return Err(Error::CeremonyNotFound);
        };
        if owner != user_id {
            return Err(Error::CeremonyNotFound);
        }
        let passkey = self
            .webauthn
            .finish_passkey_registration(credential, &state)?;
        let passkey = Passkey::new(
            credential_id(passkey.cred_id()),
            &passkey_name,
            serde_json::to_value(&passkey)?,
        )?;
        add_passkey(&self.pool, user_id, passkey).await
    }

    /// Starts a login. An unknown email gets a fake challenge that looks like
    /// a real one, so this never reveals whether the email is registered.
    pub async fn begin_login(
        &self,
        email: &str,
        now: Timestamp,
    ) -> Result<(Uuid, RequestChallengeResponse)> {
        let user = find_user_by_email(&self.pool, email).await?;
        let (options, ceremony) = match user {
            Some(user) if !user.passkeys.is_empty() => {
                let passkeys = user
                    .passkeys
                    .iter()
                    .map(webauthn_passkey)
                    .collect::<Result<Vec<_>>>()?;
                let (options, state) = self.webauthn.start_passkey_authentication(&passkeys)?;
                let ceremony = Ceremony::Login {
                    user_id: Some(user.id),
                    state: Some(state),
                };
                (options, ceremony)
            }
            _ => {
                let ceremony = Ceremony::Login {
                    user_id: None,
                    state: None,
                };
                (self.fake_login_options(email).await?, ceremony)
            }
        };
        Ok((self.start(&ceremony, now).await?, options))
    }

    /// Verifies the assertion, records the use and starts a session. Every
    /// failure is [`Error::LoginFailed`], including a missing, already-used
    /// or expired ceremony.
    pub async fn finish_login(
        &self,
        ceremony_id: Uuid,
        credential: &PublicKeyCredential,
        now: Timestamp,
    ) -> Result<(User, String)> {
        let ceremony = self.take(ceremony_id, now).await.map_err(|err| match err {
            Error::CeremonyNotFound | Error::CeremonyExpired => Error::LoginFailed,
            other => other,
        })?;
        let Ceremony::Login {
            user_id: Some(user_id),
            state: Some(state),
        } = ceremony
        else {
            return Err(Error::LoginFailed);
        };
        let result = self
            .webauthn
            .finish_passkey_authentication(credential, &state)
            .map_err(|_| Error::LoginFailed)?;
        let user = get_user(&self.pool, user_id)
            .await?
            .ok_or(Error::LoginFailed)?;
        let used_id = credential_id(result.cred_id());
        let stored = user
            .passkeys
            .iter()
            .find(|p| p.credential_id == used_id)
            .ok_or(Error::LoginFailed)?;
        let mut passkey = webauthn_passkey(stored)?;
        passkey.update_credential(&result);
        record_passkey_use(
            &self.pool,
            user_id,
            &used_id,
            serde_json::to_value(&passkey)?,
        )
        .await?;
        let session = create_session(&self.pool, user_id, now).await?;
        Ok((user, session))
    }

    async fn start(&self, ceremony: &Ceremony, now: Timestamp) -> Result<Uuid> {
        let id = Uuid::new_v4();
        let mut tx = doris_eventstore::begin(&self.pool).await?;
        sqlx::query("DELETE FROM webauthn_ceremonies WHERE expires_at <= ?")
            .bind(now.as_second())
            .execute(&mut *tx)
            .await?;
        sqlx::query(
            "INSERT INTO webauthn_ceremonies (ceremony_id, data, expires_at) VALUES (?, ?, ?)",
        )
        .bind(id.to_string())
        .bind(serde_json::to_string(ceremony)?)
        .bind((now + CEREMONY_TTL).as_second())
        .execute(&mut *tx)
        .await?;
        tx.commit().await?;
        Ok(id)
    }

    /// Removes and returns a ceremony: each one can be finished only once.
    async fn take(&self, id: Uuid, now: Timestamp) -> Result<Ceremony> {
        let row: Option<(String, i64)> = sqlx::query_as(
            "DELETE FROM webauthn_ceremonies WHERE ceremony_id = ? RETURNING data, expires_at",
        )
        .bind(id.to_string())
        .fetch_optional(&self.pool)
        .await?;
        let (data, expires_at) = row.ok_or(Error::CeremonyNotFound)?;
        if now.as_second() >= expires_at {
            return Err(Error::CeremonyExpired);
        }
        Ok(serde_json::from_str(&data)?)
    }

    /// A challenge shaped like webauthn-rs's real ones, with credential ids
    /// that are stable per email (HMAC keyed by a persisted server secret).
    async fn fake_login_options(&self, email: &str) -> Result<RequestChallengeResponse> {
        let fake_ids = self
            .fake_credential_generator
            .generate(email.trim().to_lowercase().as_bytes())?;
        let mut challenge = [0u8; 32];
        getrandom::fill(&mut challenge).expect("OS random source unavailable");
        Ok(RequestChallengeResponse {
            public_key: PublicKeyCredentialRequestOptions {
                challenge: Base64UrlSafeData::from(challenge.to_vec()),
                timeout: Some(CHALLENGE_TIMEOUT_MS),
                rp_id: self.rp_id.clone(),
                allow_credentials: fake_ids
                    .into_iter()
                    .map(|id| AllowCredentials {
                        type_: "public-key".to_owned(),
                        id: Base64UrlSafeData::from(id.to_vec()),
                        // Real passkeys registered with "none" attestation
                        // (ours) also carry no transports; webauthn-rs only
                        // keeps them for packed/TPM attestation. `None`
                        // matches real challenges here.
                        transports: None,
                    })
                    .collect(),
                user_verification: UserVerificationPolicy::Required,
                hints: None,
                extensions: None,
            },
            mediation: None,
        })
    }
}

/// Loads the persisted fake-credential HMAC key, creating it on first use.
/// Called once, at [`Auth::new`], so an unknown-email login never touches
/// the database.
async fn fake_credential_key(pool: &SqlitePool) -> Result<Vec<u8>> {
    let key = WebauthnFakeCredentialGenerator::<FakePasskeyDistribution>::new_hmac_key()?;
    sqlx::query(
        "INSERT OR IGNORE INTO server_secrets (name, value) VALUES ('fake_credential_key', ?)",
    )
    .bind(key)
    .execute(pool)
    .await?;
    Ok(
        sqlx::query_scalar("SELECT value FROM server_secrets WHERE name = 'fake_credential_key'")
            .fetch_one(pool)
            .await?,
    )
}

fn credential_id(id: &CredentialID) -> String {
    URL_SAFE_NO_PAD.encode(id.as_ref())
}

fn webauthn_passkey(passkey: &Passkey) -> Result<webauthn_rs::prelude::Passkey> {
    Ok(serde_json::from_value(passkey.passkey.clone())?)
}
