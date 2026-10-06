//! `doris.auth.v1.AuthService`: maps gRPC calls onto `doris_identity`, and
//! carries the session in an HttpOnly cookie.

use doris_identity::domain::{
    Confirmation, DomainError, Grant, Role, Scope, TokenChange, TokenRequest, User,
};
use doris_identity::{Auth, Error, SESSION_TTL};
use doris_proto::auth::v1 as pb;
use doris_proto::auth::v1::auth_service_server::AuthService;
use jiff::Timestamp;
use jiff::civil::Date;
use jiff::tz::TimeZone;
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

    /// A token's name, last day and grants as the client sent them. Every
    /// company must be one the user is a member of.
    async fn token_change(
        &self,
        user_id: Uuid,
        name: &str,
        expires_on: &str,
        grants: &[pb::TokenGrant],
    ) -> Result<TokenChange, Status> {
        let expires_at = token_expiry(expires_on, Timestamp::now())?;
        let mut parsed = Vec::with_capacity(grants.len());
        for grant in grants {
            let company_id: Uuid = grant
                .company_id
                .parse()
                .map_err(|_| Status::not_found("company_not_found"))?;
            self.member_of(user_id, company_id).await?;
            let scopes = grant
                .scopes
                .iter()
                .map(|s| Scope::parse(s))
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| Status::invalid_argument("invalid_token_grants"))?;
            parsed.push(Grant { company_id, scopes });
        }
        Ok(TokenChange {
            name: name.to_owned(),
            expires_at,
            grants: parsed,
        })
    }

    /// Membership has one source: the company module.
    async fn member_of(&self, user_id: Uuid, company_id: Uuid) -> Result<(), Status> {
        doris_company::get_company(&self.pool, company_id, user_id)
            .await
            .map_err(crate::company::status)?;
        Ok(())
    }

    /// The token request a passkey just confirmed, with every company
    /// checked again: the user may have left one meanwhile.
    async fn confirmed(
        &self,
        user_id: Uuid,
        req: &pb::FinishApiTokenRequest,
    ) -> Result<TokenRequest, Status> {
        let Confirmation::ApiToken { request } = self
            .auth
            .finish_confirmation(
                user_id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
        for grant in &request.change().grants {
            self.member_of(user_id, grant.company_id).await?;
        }
        Ok(request)
    }
}

#[tonic::async_trait]
impl AuthService for AuthApi {
    async fn get_status(
        &self,
        request: Request<pb::GetStatusRequest>,
    ) -> Result<Response<pb::GetStatusResponse>, Status> {
        let current_user = match request.extensions().get::<TokenCaller>() {
            Some(token) => Some(token.user.clone()),
            None => match session_token(&request) {
                Some(token) => doris_identity::session_user(&self.pool, &token, Timestamp::now())
                    .await
                    .map_err(status)?,
                None => None,
            },
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

    async fn continue_add_passkey(
        &self,
        request: Request<pb::ContinueAddPasskeyRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let (ceremony_id, options) = self
            .auth
            .continue_add_passkey(
                user.id,
                ceremony_id(&req.ceremony_id)?,
                &credential(&req.credential_json)?,
                Timestamp::now(),
            )
            .await
            .map_err(finish_status)?;
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

    async fn begin_create_api_token(
        &self,
        request: Request<pb::CreateApiTokenRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let change = self
            .token_change(user.id, &req.name, &req.expires_on, &req.grants)
            .await?;
        let (ceremony_id, options) = self
            .auth
            .begin_confirmation(
                user.id,
                Confirmation::ApiToken {
                    request: TokenRequest::Create { change },
                },
                Timestamp::now(),
            )
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_create_api_token(
        &self,
        request: Request<pb::FinishApiTokenRequest>,
    ) -> Result<Response<pb::CreateApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let TokenRequest::Create { change } = self.confirmed(user.id, request.get_ref()).await?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
        let (token_id, secret) = doris_identity::create_api_token(
            &self.pool,
            user.id,
            &change.name,
            change.expires_at,
            change.grants,
            Timestamp::now(),
        )
        .await
        .map_err(status)?;
        Ok(Response::new(pb::CreateApiTokenResponse {
            token_id: token_id.to_string(),
            secret,
        }))
    }

    async fn begin_change_api_token(
        &self,
        request: Request<pb::ChangeApiTokenRequest>,
    ) -> Result<Response<pb::BeginCeremonyResponse>, Status> {
        let user = self.user(&request).await?;
        let req = request.into_inner();
        let token_id = api_token_id(&req.token_id)?;
        let change = self
            .token_change(user.id, &req.name, &req.expires_on, &req.grants)
            .await?;
        let (ceremony_id, options) = self
            .auth
            .begin_confirmation(
                user.id,
                Confirmation::ApiToken {
                    request: TokenRequest::Change { token_id, change },
                },
                Timestamp::now(),
            )
            .await
            .map_err(status)?;
        ceremony_response(ceremony_id, &options)
    }

    async fn finish_change_api_token(
        &self,
        request: Request<pb::FinishApiTokenRequest>,
    ) -> Result<Response<pb::ChangeApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let TokenRequest::Change { token_id, change } =
            self.confirmed(user.id, request.get_ref()).await?
        else {
            return Err(Status::failed_precondition("ceremony_expired"));
        };
        doris_identity::change_api_token(&self.pool, user.id, token_id, change, Timestamp::now())
            .await
            .map_err(status)?;
        Ok(Response::new(pb::ChangeApiTokenResponse {}))
    }

    async fn list_api_tokens(
        &self,
        request: Request<pb::ListApiTokensRequest>,
    ) -> Result<Response<pb::ListApiTokensResponse>, Status> {
        let user = self.user(&request).await?;
        let tokens = doris_identity::list_api_tokens(&self.pool, user.id)
            .await
            .map_err(status)?
            .into_iter()
            .map(|t| pb::ApiToken {
                id: t.id.to_string(),
                name: t.name,
                grants: t.grants.iter().map(grant_message).collect(),
                created_at: t.created_at,
                expires_at: t.expires_at.to_string(),
                last_used_at: t.last_used_at.map(|at| at.to_string()),
                revoked_at: t.revoked_at,
            })
            .collect();
        Ok(Response::new(pb::ListApiTokensResponse { tokens }))
    }

    async fn revoke_api_token(
        &self,
        request: Request<pb::RevokeApiTokenRequest>,
    ) -> Result<Response<pb::RevokeApiTokenResponse>, Status> {
        let user = self.user(&request).await?;
        let token_id = api_token_id(&request.get_ref().token_id)?;
        doris_identity::revoke_api_token(&self.pool, user.id, token_id)
            .await
            .map_err(status)?;
        Ok(Response::new(pb::RevokeApiTokenResponse {}))
    }
}

/// The signed-in user, or `Unauthenticated`. Shared by every service.
pub(crate) async fn signed_in_user<T>(
    pool: &SqlitePool,
    request: &Request<T>,
) -> Result<User, Status> {
    if let Some(token) = request.extensions().get::<TokenCaller>() {
        return Ok(token.user.clone());
    }
    session_user(pool, request.metadata().as_ref()).await
}

/// A call made with an API token. `auth_gate` puts it in the request.
#[derive(Clone)]
pub(crate) struct TokenCaller {
    pub user: User,
    pub access: doris_identity::TokenAccess,
    pub required: crate::access::Access,
}

/// The token in an `authorization: Bearer …` header. The scheme is
/// case-insensitive. Another scheme (a proxy's `Basic`) is not ours and
/// leaves the cookie in charge; a `Bearer` with an empty or garbled token
/// is returned and fails as a token, never falling back to the cookie.
pub(crate) fn bearer(headers: &http::HeaderMap) -> Option<String> {
    let value = headers.get(http::header::AUTHORIZATION)?;
    let value = value.to_str().unwrap_or_default().trim();
    let (scheme, rest) = value.split_once(' ').unwrap_or((value, ""));
    scheme
        .eq_ignore_ascii_case("bearer")
        .then(|| rest.trim().to_owned())
}

/// The company asked about and the caller. A token needs the call's scope
/// for that company; membership is then checked by each module.
pub(crate) async fn company_caller<T>(
    pool: &SqlitePool,
    request: &Request<T>,
    company_id: &str,
) -> Result<(Uuid, Uuid), Status> {
    let user = signed_in_user(pool, request).await?;
    let company: Uuid = company_id
        .parse()
        .map_err(|_| Status::not_found("company_not_found"))?;
    if let Some(token) = request.extensions().get::<TokenCaller>() {
        let grant = token
            .access
            .grants
            .iter()
            .find(|g| g.company_id == company)
            .ok_or_else(|| Status::not_found("company_not_found"))?;
        match token.required {
            crate::access::Access::Company(scope) if grant.scopes.contains(&scope) => {}
            _ => return Err(Status::permission_denied("missing_scope")),
        }
    }
    Ok((company, user.id))
}

/// The user whose session cookie is in `headers`, or `Unauthenticated`.
pub(crate) async fn session_user(
    pool: &SqlitePool,
    headers: &http::HeaderMap,
) -> Result<User, Status> {
    let token = cookie_session(headers).ok_or_else(not_signed_in)?;
    doris_identity::session_user(pool, &token, Timestamp::now())
        .await
        .map_err(status)?
        .ok_or_else(not_signed_in)
}

fn grant_message(grant: &Grant) -> pb::TokenGrant {
    pb::TokenGrant {
        company_id: grant.company_id.to_string(),
        scopes: grant.scopes.iter().map(|s| s.as_str().to_owned()).collect(),
    }
}

fn api_token_id(raw: &str) -> Result<Uuid, Status> {
    raw.parse()
        .map_err(|_| Status::not_found("api_token_not_found"))
}

/// When a token whose last day is `raw` (`YYYY-MM-DD`) stops working:
/// midnight in Sweden after that day. The day is today at the earliest and
/// 366 days off at most.
fn token_expiry(raw: &str, now: Timestamp) -> Result<Timestamp, Status> {
    use jiff::ToSpan;
    let invalid = || Status::invalid_argument("invalid_token_expiry");
    let last_day: Date = raw.parse().map_err(|_| invalid())?;
    let today = today_in_sweden(now);
    let latest = today.checked_add(366.days()).map_err(|_| invalid())?;
    if last_day < today || last_day > latest {
        return Err(invalid());
    }
    let sweden = TimeZone::get("Europe/Stockholm").expect("bundled tz database");
    let midnight = last_day.tomorrow().map_err(|_| invalid())?;
    Ok(midnight
        .to_zoned(sweden)
        .map_err(|_| invalid())?
        .timestamp())
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
    cookie_session(request.metadata().as_ref())
}

fn cookie_session(headers: &http::HeaderMap) -> Option<String> {
    headers
        .get_all(http::header::COOKIE)
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

pub(crate) fn not_signed_in() -> Status {
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
        Error::Domain(DomainError::NotTokenOwner) | Error::ApiTokenNotFound => {
            Status::not_found("api_token_not_found")
        }
        Error::Domain(DomainError::TokenRevoked) => {
            Status::failed_precondition("api_token_revoked")
        }
        Error::Domain(err) => Status::invalid_argument(domain_code(err)),
        Error::AlreadyExists => Status::already_exists("already_exists"),
        Error::InvitationNotFound => Status::not_found("invitation_not_found"),
        Error::UserNotFound => Status::not_found("user_not_found"),
        Error::CeremonyNotFound | Error::CeremonyExpired => {
            Status::failed_precondition("ceremony_expired")
        }
        Error::LoginFailed => Status::unauthenticated("login_failed"),
        Error::CredentialRejected => Status::invalid_argument("credential_rejected"),
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
        DomainError::InvalidTokenName => "invalid_token_name",
        DomainError::InvalidTokenExpiry => "invalid_token_expiry",
        DomainError::InvalidTokenGrants => "invalid_token_grants",
        DomainError::NotTokenOwner => "api_token_not_found",
        DomainError::TokenRevoked => "api_token_revoked",
    }
}

/// Today's date for date rules: Swedish, like the browser's local date.
pub(crate) fn today() -> Date {
    today_in_sweden(Timestamp::now())
}

/// The time now in Sweden, without a zone: the AGI file's Skapad.
pub(crate) fn now() -> jiff::civil::DateTime {
    let sweden = TimeZone::get("Europe/Stockholm").expect("bundled tz database");
    Timestamp::now().to_zoned(sweden).datetime()
}

/// The date in Sweden at `ts`.
fn today_in_sweden(ts: Timestamp) -> Date {
    // The tz database is bundled (jiff `tzdb-bundle-always`), so this holds
    // even without tzdata on the host.
    let sweden = TimeZone::get("Europe/Stockholm").expect("bundled tz database");
    ts.to_zoned(sweden).date()
}

#[cfg(test)]
mod tests {
    use super::{today_in_sweden, token_expiry};
    use jiff::civil::date;

    #[test]
    fn a_token_ends_at_midnight_in_sweden_after_its_last_day() {
        let at = |s: &str| -> jiff::Timestamp { s.parse().unwrap() };
        let now = at("2026-10-06T10:00:00Z");
        // The last day today: the rest of today, until midnight in Sweden (summer time).
        assert_eq!(
            token_expiry("2026-10-06", now).unwrap(),
            at("2026-10-06T22:00:00Z")
        );
        // In winter time, midnight is 23:00Z.
        assert_eq!(
            token_expiry("2026-12-01", now).unwrap(),
            at("2026-12-01T23:00:00Z")
        );
        // The day before summer time starts ends at 23:00Z too.
        assert_eq!(
            token_expiry("2027-03-27", now).unwrap(),
            at("2027-03-27T23:00:00Z")
        );
        // Today counts in Sweden: at 23:30Z on the 6th it is already the 7th.
        assert!(token_expiry("2026-10-06", at("2026-10-06T23:30:00Z")).is_err());
        assert!(token_expiry("2027-10-07", now).is_ok());
        assert!(token_expiry("2027-10-08", now).is_err());
        assert!(token_expiry("2026-10-05", now).is_err());
        assert!(token_expiry("i morgon", now).is_err());
    }

    #[test]
    fn today_is_the_date_in_sweden() {
        // Summer time (UTC+2) and winter time (UTC+1): past midnight in Sweden.
        let at = |s: &str| s.parse().unwrap();
        assert_eq!(
            today_in_sweden(at("2026-10-01T22:30:00Z")),
            date(2026, 10, 2)
        );
        assert_eq!(
            today_in_sweden(at("2026-01-01T23:30:00Z")),
            date(2026, 1, 2)
        );
        assert_eq!(
            today_in_sweden(at("2026-01-01T22:30:00Z")),
            date(2026, 1, 1)
        );
    }
}
