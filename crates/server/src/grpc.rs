//! `doris.auth.v1.AuthService`: maps gRPC calls onto `doris_identity`, and
//! carries the session in an HttpOnly cookie.

use doris_identity::domain::{DomainError, Role, User};
use doris_identity::{Auth, Error, SESSION_TTL};
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_server::AuthService;
use jiff::Timestamp;
use sqlx::SqlitePool;
use tonic::{Request, Response, Status};
use uuid::Uuid;

pub const SESSION_COOKIE: &str = "doris_session";

pub struct AuthApi {
    pool: SqlitePool,
    auth: Auth,
}

impl AuthApi {
    pub fn new(pool: SqlitePool, auth: Auth) -> Self {
        Self { pool, auth }
    }

    /// The signed-in user, or `Unauthenticated`.
    async fn user<T>(&self, request: &Request<T>) -> Result<User, Status> {
        signed_in_user(&self.pool, request).await
    }

    async fn admin<T>(&self, request: &Request<T>) -> Result<User, Status> {
        let user = self.user(request).await?;
        match user.role {
            Role::Admin => Ok(user),
            Role::Member => Err(Status::permission_denied("not_admin")),
        }
    }
}

#[tonic::async_trait]
impl AuthService for AuthApi {
    async fn get_status(
        &self,
        request: Request<pb::GetStatusRequest>,
    ) -> Result<Response<pb::GetStatusResponse>, Status> {
        let current_user = match session_token(&request) {
            Some(token) => doris_identity::session_user(&self.pool, &token, Timestamp::now())
                .await
                .map_err(status)?,
            None => None,
        };
        Ok(Response::new(pb::GetStatusResponse {
            bootstrap_required: doris_identity::bootstrap_required(&self.pool)
                .await
                .map_err(status)?,
            current_user: current_user.as_ref().map(user_message),
        }))
    }

    async fn begin_registration(
        &self,
        request: Request<pb::BeginRegistrationRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let req = request.into_inner();
        let (ceremony_id, options) = self
            .auth
            .begin_registration(
                &req.email,
                &req.display_name,
                req.invitation_token.as_deref(),
                &req.passkey_name,
                Timestamp::now(),
            )
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_registration(
        &self,
        request: Request<pb::FinishRegistrationRequest>,
    ) -> Result<Response<pb::User>, Status> {
        let req = request.into_inner();
        let (user, session) = self
            .auth
            .finish_registration(
                ceremony_id(&req.ceremony_id)?,
                req.invitation_token.as_deref(),
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(with_session_cookie(user_message(&user), &session))
    }

    async fn begin_login(
        &self,
        request: Request<pb::BeginLoginRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let req = request.into_inner();
        let (ceremony_id, options) = self
            .auth
            .begin_login(&req.email, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_login(
        &self,
        request: Request<pb::FinishLoginRequest>,
    ) -> Result<Response<pb::User>, Status> {
        let req = request.into_inner();
        let (user, session) = self
            .auth
            .finish_login(
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(with_session_cookie(user_message(&user), &session))
    }

    async fn logout(
        &self,
        request: Request<pb::LogoutRequest>,
    ) -> Result<Response<pb::LogoutResponse>, Status> {
        if let Some(token) = session_token(&request) {
            doris_identity::end_session(&self.pool, &token)
                .await
                .map_err(status)?;
        }
        let mut response = Response::new(pb::LogoutResponse {});
        set_cookie(&mut response, "", 0);
        Ok(response)
    }

    async fn begin_add_passkey(
        &self,
        request: Request<pb::BeginAddPasskeyRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let (ceremony_id, options) = self
            .auth
            .begin_add_passkey(user.id, &request.get_ref().passkey_name, Timestamp::now())
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_add_passkey(
        &self,
        request: Request<pb::FinishAddPasskeyRequest>,
    ) -> Result<Response<pb::FinishAddPasskeyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        self.auth
            .finish_add_passkey(
                user.id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
        Ok(Response::new(pb::FinishAddPasskeyResponse {}))
    }

    async fn list_passkeys(
        &self,
        request: Request<pb::ListPasskeysRequest>,
    ) -> Result<Response<pb::ListPasskeysResponse>, Status> {
        let user = self.user(&request).await?;
        let passkeys = doris_identity::list_passkeys(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|p| pb::Passkey {
                credential_id: p.credential_id,
                name: p.name,
                added_at: p.added_at,
                last_used_at: p.last_used_at,
            })
            .collect();
        Ok(Response::new(pb::ListPasskeysResponse { passkeys }))
    }

    async fn get_invitation(
        &self,
        request: Request<pb::GetInvitationRequest>,
    ) -> Result<Response<pb::GetInvitationResponse>, Status> {
        let email = doris_identity::invitation_email(
            &self.pool,
            &request.get_ref().token,
            Timestamp::now(),
        )
        .await
        .map_err(status)?
        .ok_or_else(|| Status::not_found("invitation_not_found"))?;
        Ok(Response::new(pb::GetInvitationResponse {
            email: email.as_str().to_owned(),
        }))
    }

    async fn create_invitation(
        &self,
        request: Request<pb::CreateInvitationRequest>,
    ) -> Result<Response<pb::CreateInvitationResponse>, Status> {
        let admin = self.admin(&request).await?;
        let now = Timestamp::now();
        let (_, token) =
            doris_identity::create_invitation(&self.pool, admin.id, &request.get_ref().email, now)
                .await
                .map_err(status)?;
        Ok(Response::new(pb::CreateInvitationResponse {
            token,
            expires_at: (now + doris_identity::domain::INVITATION_TTL).to_string(),
        }))
    }

    async fn list_invitations(
        &self,
        request: Request<pb::ListInvitationsRequest>,
    ) -> Result<Response<pb::ListInvitationsResponse>, Status> {
        self.admin(&request).await?;
        let invitations = doris_identity::list_invitations(&self.pool)
            .await
            .map_err(status)?
            .into_iter()
            .map(|i| pb::Invitation {
                id: i.id.to_string(),
                email: i.email,
                expires_at: i.expires_at.to_string(),
                accepted: i.accepted,
            })
            .collect();
        Ok(Response::new(pb::ListInvitationsResponse { invitations }))
    }
}

/// The signed-in user, or `Unauthenticated`. Shared by every service.
pub(crate) async fn signed_in_user<T>(
    pool: &SqlitePool,
    request: &Request<T>,
) -> Result<User, Status> {
    let token = session_token(request).ok_or_else(not_signed_in)?;
    doris_identity::session_user(pool, &token, Timestamp::now())
        .await
        .map_err(status)?
        .ok_or_else(not_signed_in)
}

fn user_message(user: &User) -> pb::User {
    pb::User {
        id: user.id.to_string(),
        email: user.email.as_str().to_owned(),
        display_name: user.display_name.as_str().to_owned(),
        role: match user.role {
            Role::Admin => pb::Role::Admin,
            Role::Member => pb::Role::Member,
        }
        .into(),
    }
}

fn ceremony_response<T: serde::Serialize>(
    ceremony_id: Uuid,
    options: &T,
) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
    Ok(Response::new(pb::BeginCeremonyResponse {
        ceremony_id: ceremony_id.to_string(),
        options_json: serde_json::to_string(options).map_err(|_| Status::internal("internal"))?,
    }))
}

fn ceremony_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::invalid_argument("invalid_ceremony"))
}

fn credential<T: serde::de::DeserializeOwned>(json: &str) -> Result<T, Status> {
    serde_json::from_str(json).map_err(|_| Status::invalid_argument("invalid_credential"))
}

fn session_token<T>(request: &Request<T>) -> Option<String> {
    request
        .metadata()
        .get_all("cookie")
        .iter()
        .filter_map(|value| value.to_str().ok())
        .flat_map(|header| header.split(';'))
        .find_map(|cookie| {
            cookie
                .trim()
                .strip_prefix(SESSION_COOKIE)
                .and_then(|rest| rest.strip_prefix('='))
                .filter(|token| !token.is_empty())
                .map(str::to_owned)
        })
}

fn with_session_cookie<T>(message: T, token: &str) -> Response<T> {
    let mut response = Response::new(message);
    set_cookie(&mut response, token, SESSION_TTL.as_secs());
    response
}

fn set_cookie<T>(response: &mut Response<T>, token: &str, max_age: i64) {
    let cookie = format!(
        "{SESSION_COOKIE}={token}; HttpOnly; Secure; SameSite=Strict; Path=/; Max-Age={max_age}"
    );
    response.metadata_mut().insert(
        "set-cookie",
        cookie.parse().expect("cookie is valid header ascii"),
    );
}

fn not_signed_in() -> Status {
    Status::unauthenticated("not_signed_in")
}

/// Like [`status`], but a rejected WebAuthn credential is the client's fault.
fn finish_status(err: Error) -> Status {
    match err {
        Error::Webauthn(_) => Status::invalid_argument("credential_rejected"),
        other => status(other),
    }
}

/// Maps identity errors to gRPC statuses. Messages are stable codes the
/// frontend translates; they never contain personal data.
pub(crate) fn status(err: Error) -> Status {
    match err {
        Error::Domain(DomainError::NotAdmin) => Status::permission_denied("not_admin"),
        Error::Domain(err) => Status::invalid_argument(domain_code(err)),
        Error::AlreadyExists => Status::already_exists("already_exists"),
        Error::InvitationNotFound => Status::not_found("invitation_not_found"),
        Error::UserNotFound => Status::not_found("user_not_found"),
        Error::CeremonyNotFound | Error::CeremonyExpired => {
            Status::failed_precondition("ceremony_expired")
        }
        Error::LoginFailed => Status::unauthenticated("login_failed"),
        Error::Webauthn(err) => {
            tracing::error!("webauthn: {err}");
            Status::internal("internal")
        }
        Error::Store(err) => {
            tracing::error!("store: {err}");
            Status::internal("internal")
        }
    }
}

fn domain_code(err: DomainError) -> &'static str {
    match err {
        DomainError::InvalidEmail => "invalid_email",
        DomainError::InvalidDisplayName => "invalid_display_name",
        DomainError::InvalidPasskeyName => "invalid_passkey_name",
        DomainError::InvitationRequired => "invitation_required",
        DomainError::InvitationExpired => "invitation_expired",
        DomainError::InvitationAlreadyUsed => "invitation_already_used",
        DomainError::InvitationEmailMismatch => "invitation_email_mismatch",
        DomainError::DuplicatePasskey => "duplicate_passkey",
        DomainError::UnknownPasskey => "unknown_passkey",
        DomainError::NotAdmin => "not_admin",
    }
}
