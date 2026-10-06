use doris_identity::domain::{DomainError, Grant, Role, Scope, TokenChange, TokenRequest, User};
use doris_identity::{Auth, CEREMONY_TTL, Error, create_invitation, get_user, session_user};
use jiff::{SignedDuration, Timestamp};
use sqlx::SqlitePool;
use url::Url;
use webauthn_authenticator_rs::WebauthnAuthenticator;
use webauthn_authenticator_rs::softpasskey::SoftPasskey;

type Authenticator = WebauthnAuthenticator<SoftPasskey>;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn origin() -> Url {
    Url::parse("http://localhost:3000").unwrap()
}

fn authenticator() -> Authenticator {
    WebauthnAuthenticator::new(SoftPasskey::new(true))
}

async fn setup() -> (SqlitePool, Auth) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let auth = Auth::new(pool.clone(), "localhost", &origin())
        .await
        .unwrap();
    (pool, auth)
}

async fn sign_up(
    auth: &Auth,
    device: &mut Authenticator,
    email: &str,
    token: Option<&str>,
) -> (User, String) {
    let (ceremony, options) = auth
        .begin_registration(email, "Anna", token, "Laptop", now())
        .await
        .unwrap();
    let credential = device.do_registration(origin(), options).unwrap();
    auth.finish_registration(ceremony, token, &credential, now())
        .await
        .unwrap()
}

async fn log_in(
    auth: &Auth,
    device: &mut Authenticator,
    email: &str,
) -> Result<(User, String), Error> {
    let (ceremony, options) = auth.begin_login(email, now()).await.unwrap();
    let credential = device.do_authentication(origin(), options).unwrap();
    auth.finish_login(ceremony, &credential, now()).await
}

#[tokio::test]
async fn registering_with_a_passkey_creates_the_admin_and_a_session() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();

    let (anna, session) = sign_up(&auth, &mut laptop, "Anna@Example.se", None).await;

    assert_eq!(anna.role, Role::Admin);
    assert_eq!(anna.email.as_str(), "anna@example.se");
    assert_eq!(anna.passkeys.len(), 1);
    assert_eq!(anna.passkeys[0].name, "Laptop");
    assert_eq!(
        session_user(&pool, &session, now()).await.unwrap(),
        Some(anna)
    );
}

#[tokio::test]
async fn logging_in_with_the_passkey_starts_a_new_session() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let (anna, first) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (user, second) = log_in(&auth, &mut laptop, "ANNA@example.se").await.unwrap();

    assert_eq!(user.id, anna.id);
    assert_ne!(first, second);
    assert_eq!(
        session_user(&pool, &second, now())
            .await
            .unwrap()
            .unwrap()
            .id,
        anna.id
    );
}

#[tokio::test]
async fn logging_in_updates_the_stored_credential() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let (anna, _) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    log_in(&auth, &mut laptop, "anna@example.se").await.unwrap();

    let after = get_user(&pool, anna.id).await.unwrap().unwrap();
    assert_ne!(after.passkeys[0].passkey, anna.passkeys[0].passkey);
    let last_used: Option<String> = sqlx::query_scalar("SELECT last_used_at FROM passkeys")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert!(last_used.is_some());
}

#[tokio::test]
async fn registration_is_refused_before_the_authenticator_is_asked() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let refuse = |email: &'static str, token: Option<String>, passkey_name: &'static str| {
        let auth = &auth;
        async move {
            auth.begin_registration(email, "Bo", token.as_deref(), passkey_name, now())
                .await
                .unwrap_err()
        }
    };

    let uninvited = refuse("bo@example.se", None, "Laptop").await;
    let other_email = refuse("cecilia@example.se", Some(token.clone()), "Laptop").await;
    let bad_passkey_name = refuse("bo@example.se", Some(token.clone()), " ").await;
    let bad_email = refuse("bo", Some(token), "Laptop").await;

    assert!(
        matches!(uninvited, Error::Domain(DomainError::InvitationRequired)),
        "{uninvited:?}"
    );
    assert!(
        matches!(
            other_email,
            Error::Domain(DomainError::InvitationEmailMismatch)
        ),
        "{other_email:?}"
    );
    assert!(
        matches!(
            bad_passkey_name,
            Error::Domain(DomainError::InvalidPasskeyName)
        ),
        "{bad_passkey_name:?}"
    );
    assert!(
        matches!(bad_email, Error::Domain(DomainError::InvalidEmail)),
        "{bad_email:?}"
    );
    let ceremonies: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(ceremonies, 0);
}

#[tokio::test]
async fn an_invited_user_registers_with_their_own_passkey() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let mut phone = authenticator();

    let (bo, _) = sign_up(&auth, &mut phone, "bo@example.se", Some(&token)).await;
    let (logged_in, _) = log_in(&auth, &mut phone, "bo@example.se").await.unwrap();

    assert_eq!(bo.role, Role::Member);
    assert_eq!(logged_in.id, bo.id);
}

#[tokio::test]
async fn a_ceremony_can_be_finished_only_once() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    let (ceremony, options) = auth
        .begin_registration("anna@example.se", "Anna", None, "Laptop", now())
        .await
        .unwrap();
    let credential = laptop.do_registration(origin(), options).unwrap();
    auth.finish_registration(ceremony, None, &credential, now())
        .await
        .unwrap();

    let again = auth
        .finish_registration(ceremony, None, &credential, now())
        .await
        .unwrap_err();

    assert!(matches!(again, Error::CeremonyNotFound), "{again:?}");
}

#[tokio::test]
async fn a_ceremony_expires_after_five_minutes() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    let (ceremony, options) = auth
        .begin_registration("anna@example.se", "Anna", None, "Laptop", now())
        .await
        .unwrap();
    let credential = laptop.do_registration(origin(), options).unwrap();

    let late = auth
        .finish_registration(ceremony, None, &credential, now() + CEREMONY_TTL)
        .await
        .unwrap_err();

    assert_eq!(CEREMONY_TTL, jiff::SignedDuration::from_mins(5));
    assert!(matches!(late, Error::CeremonyExpired), "{late:?}");
}

#[tokio::test]
async fn another_users_passkey_cannot_finish_a_login() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let mut bos = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&token)).await;

    let (annas_login, _) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let (_, bos_options) = auth.begin_login("bo@example.se", now()).await.unwrap();
    let bos_assertion = bos.do_authentication(origin(), bos_options).unwrap();
    let err = auth
        .finish_login(annas_login, &bos_assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}

#[tokio::test]
async fn an_unknown_email_gets_a_convincing_fake_challenge() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (_, real) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let (fake_ceremony, fake) = auth.begin_login("nobody@example.se", now()).await.unwrap();
    let (_, fake_again) = auth.begin_login(" NOBODY@example.se", now()).await.unwrap();
    let restarted = Auth::new(pool.clone(), "localhost", &origin())
        .await
        .unwrap();
    let (_, after_restart) = restarted
        .begin_login("nobody@example.se", now())
        .await
        .unwrap();

    let real = serde_json::to_value(&real).unwrap();
    let [fake, fake_again, after_restart] =
        [fake, fake_again, after_restart].map(|o| serde_json::to_value(o).unwrap());
    let keys = |v: &serde_json::Value| {
        let mut keys: Vec<String> = v["publicKey"]
            .as_object()
            .unwrap()
            .keys()
            .cloned()
            .collect();
        keys.sort();
        keys
    };
    assert_eq!(keys(&fake), keys(&real));
    for field in ["timeout", "rpId", "userVerification"] {
        assert_eq!(
            fake["publicKey"][field], real["publicKey"][field],
            "{field}"
        );
    }
    assert_ne!(
        fake["publicKey"]["challenge"],
        fake_again["publicKey"]["challenge"]
    );
    assert_eq!(
        fake["publicKey"]["allowCredentials"],
        fake_again["publicKey"]["allowCredentials"]
    );
    assert_eq!(
        fake["publicKey"]["allowCredentials"],
        after_restart["publicKey"]["allowCredentials"]
    );

    let (_, real_options) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let assertion = laptop.do_authentication(origin(), real_options).unwrap();
    let err = auth
        .finish_login(fake_ceremony, &assertion, now())
        .await
        .unwrap_err();
    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}

#[tokio::test]
async fn a_login_ceremony_can_be_finished_only_once() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    sign_up(&auth, &mut laptop, "anna@example.se", None).await;
    let (ceremony, options) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let assertion = laptop.do_authentication(origin(), options).unwrap();
    auth.finish_login(ceremony, &assertion, now())
        .await
        .unwrap();

    let again = auth
        .finish_login(ceremony, &assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(again, Error::LoginFailed), "{again:?}");
}

#[tokio::test]
async fn a_login_ceremony_expires_after_five_minutes() {
    let (_, auth) = setup().await;
    let mut laptop = authenticator();
    sign_up(&auth, &mut laptop, "anna@example.se", None).await;
    let (ceremony, options) = auth.begin_login("anna@example.se", now()).await.unwrap();
    let assertion = laptop.do_authentication(origin(), options).unwrap();

    let late = auth
        .finish_login(ceremony, &assertion, now() + CEREMONY_TTL)
        .await
        .unwrap_err();

    assert!(matches!(late, Error::LoginFailed), "{late:?}");
}

#[tokio::test]
async fn unknown_email_logins_do_not_write_to_the_database() {
    let (pool, auth) = setup().await;

    sqlx::query("DROP TABLE server_secrets")
        .execute(&pool)
        .await
        .unwrap();

    auth.begin_login("nobody@example.se", now()).await.unwrap();
}

#[tokio::test]
async fn finish_registration_rechecks_the_invitation() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token_bo) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let (_, token_cecilia) = create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap();

    let (ceremony, options) = auth
        .begin_registration("bo@example.se", "Bo", Some(&token_bo), "Laptop", now())
        .await
        .unwrap();
    let credential = authenticator().do_registration(origin(), options).unwrap();
    let wrong_token = auth
        .finish_registration(ceremony, Some(&token_cecilia), &credential, now())
        .await
        .unwrap_err();
    assert!(
        matches!(
            wrong_token,
            Error::Domain(DomainError::InvitationEmailMismatch)
        ),
        "{wrong_token:?}"
    );

    let (ceremony, options) = auth
        .begin_registration("bo@example.se", "Bo", Some(&token_bo), "Laptop", now())
        .await
        .unwrap();
    let credential = authenticator().do_registration(origin(), options).unwrap();
    let no_token = auth
        .finish_registration(ceremony, None, &credential, now())
        .await
        .unwrap_err();
    assert!(
        matches!(no_token, Error::Domain(DomainError::InvitationRequired)),
        "{no_token:?}"
    );
}

#[tokio::test]
async fn a_user_can_add_a_second_passkey_and_log_in_with_either() {
    let (pool, auth) = setup().await;
    let mut laptop = authenticator();
    let mut phone = authenticator();
    let (anna, _) = sign_up(&auth, &mut laptop, "anna@example.se", None).await;

    let (ceremony, options) = auth
        .begin_add_passkey(anna.id, "Telefon", now())
        .await
        .unwrap();
    let credential = phone.do_registration(origin(), options).unwrap();
    auth.finish_add_passkey(anna.id, ceremony, &credential, now())
        .await
        .unwrap();

    let names: Vec<String> = get_user(&pool, anna.id)
        .await
        .unwrap()
        .unwrap()
        .passkeys
        .into_iter()
        .map(|p| p.name)
        .collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert_eq!(
        log_in(&auth, &mut phone, "anna@example.se")
            .await
            .unwrap()
            .0
            .id,
        anna.id
    );
    assert_eq!(
        log_in(&auth, &mut laptop, "anna@example.se")
            .await
            .unwrap()
            .0
            .id,
        anna.id
    );
}

#[tokio::test]
async fn an_add_passkey_ceremony_belongs_to_the_user_who_started_it() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let (bo, _) = sign_up(&auth, &mut authenticator(), "bo@example.se", Some(&token)).await;

    let (ceremony, options) = auth
        .begin_add_passkey(anna.id, "Telefon", now())
        .await
        .unwrap();
    let credential = authenticator().do_registration(origin(), options).unwrap();
    let err = auth
        .finish_add_passkey(bo.id, ceremony, &credential, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::CeremonyNotFound), "{err:?}");
    assert_eq!(
        get_user(&pool, anna.id)
            .await
            .unwrap()
            .unwrap()
            .passkeys
            .len(),
        1
    );
}

fn new_token() -> TokenRequest {
    TokenRequest::Create {
        change: TokenChange {
            name: "Agent".into(),
            expires_at: now() + SignedDuration::from_hours(24),
            grants: vec![Grant {
                company_id: uuid::Uuid::new_v4(),
                scopes: vec![Scope::LedgerRead],
            }],
        },
    }
}

async fn ceremonies(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM webauthn_ceremonies")
        .fetch_one(pool)
        .await
        .unwrap()
}

async fn passkey_uses(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE event_type = 'PasskeyUsed'")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn a_passkey_confirms_a_token_request() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;

    let (ceremony, options) = auth
        .begin_api_token(anna.id, new_token(), now())
        .await
        .unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let confirmed = auth
        .finish_api_token(anna.id, ceremony, &assertion, now())
        .await
        .unwrap();

    assert!(matches!(&confirmed, TokenRequest::Create { change } if change.name == "Agent"));
    assert_eq!(
        passkey_uses(&pool).await,
        1,
        "the use (and counter) is recorded"
    );
    assert_eq!(ceremonies(&pool).await, 0);
}

#[tokio::test]
async fn a_token_request_is_refused_before_the_authenticator_is_asked() {
    let (pool, auth) = setup().await;
    let (anna, _) = sign_up(&auth, &mut authenticator(), "anna@example.se", None).await;
    let mut bad = new_token();
    if let TokenRequest::Create { change } = &mut bad {
        change.name = " ".into();
    }

    let err = auth.begin_api_token(anna.id, bad, now()).await.unwrap_err();
    let someone_elses = auth
        .begin_api_token(
            anna.id,
            TokenRequest::Change {
                token_id: uuid::Uuid::new_v4(),
                change: new_token().change().clone(),
            },
            now(),
        )
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Domain(DomainError::InvalidTokenName)),
        "{err:?}"
    );
    assert!(
        matches!(someone_elses, Error::ApiTokenNotFound),
        "{someone_elses:?}"
    );
    assert_eq!(ceremonies(&pool).await, 0);
}

#[tokio::test]
async fn only_the_users_own_passkey_confirms_their_token_request() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let mut bos = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&invitation)).await;

    let (ceremony, _) = auth
        .begin_api_token(anna.id, new_token(), now())
        .await
        .unwrap();
    let (_, bos_options) = auth.begin_login("bo@example.se", now()).await.unwrap();
    let bos_assertion = bos.do_authentication(origin(), bos_options).unwrap();
    let err = auth
        .finish_api_token(anna.id, ceremony, &bos_assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::CredentialRejected), "{err:?}");
}

#[tokio::test]
async fn a_token_ceremony_is_finished_once_by_the_user_who_began_it_within_five_minutes() {
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let (bo, _) = sign_up(
        &auth,
        &mut authenticator(),
        "bo@example.se",
        Some(&invitation),
    )
    .await;

    // Finished by someone else: gone, as if it never was.
    let (ceremony, options) = auth
        .begin_api_token(anna.id, new_token(), now())
        .await
        .unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let by_bo = auth
        .finish_api_token(bo.id, ceremony, &assertion, now())
        .await
        .unwrap_err();
    let again = auth
        .finish_api_token(anna.id, ceremony, &assertion, now())
        .await
        .unwrap_err();

    // Finished too late.
    let (late, options) = auth
        .begin_api_token(anna.id, new_token(), now())
        .await
        .unwrap();
    let assertion = annas.do_authentication(origin(), options).unwrap();
    let expired = auth
        .finish_api_token(anna.id, late, &assertion, now() + CEREMONY_TTL)
        .await
        .unwrap_err();

    assert!(matches!(by_bo, Error::CeremonyNotFound), "{by_bo:?}");
    assert!(matches!(again, Error::CeremonyNotFound), "{again:?}");
    assert!(matches!(expired, Error::CeremonyExpired), "{expired:?}");
}

#[tokio::test]
async fn logging_in_still_fails_the_same_way_for_a_wrong_passkey() {
    // finish_login now shares the assertion check with token ceremonies.
    let (pool, auth) = setup().await;
    let mut annas = authenticator();
    let (anna, _) = sign_up(&auth, &mut annas, "anna@example.se", None).await;
    let (_, invitation) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let mut bos = authenticator();
    sign_up(&auth, &mut bos, "bo@example.se", Some(&invitation)).await;
    let (_, token_options) = auth
        .begin_api_token(anna.id, new_token(), now())
        .await
        .unwrap();
    let annas_assertion = annas.do_authentication(origin(), token_options).unwrap();
    let (bos_login, _) = auth.begin_login("bo@example.se", now()).await.unwrap();

    let err = auth
        .finish_login(bos_login, &annas_assertion, now())
        .await
        .unwrap_err();

    assert!(matches!(err, Error::LoginFailed), "{err:?}");
}
