use doris_identity::domain::{DomainError, Passkey, Role};
use doris_identity::{
    Error, add_passkey, create_invitation, find_user_by_email, get_user, rebuild_projections,
    record_passkey_use, register,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str) -> Passkey {
    Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap()
}

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

async fn event_count(pool: &SqlitePool) -> i64 {
    sqlx::query_scalar("SELECT COUNT(*) FROM events")
        .fetch_one(pool)
        .await
        .unwrap()
}

#[tokio::test]
async fn first_user_becomes_admin_and_is_findable_by_email() {
    let pool = db().await;

    let anna = register(&pool, "Anna@Example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    assert_eq!(anna.role, Role::Admin);
    let found = find_user_by_email(&pool, "ANNA@example.se")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found, anna);
    assert_eq!(get_user(&pool, anna.id).await.unwrap(), Some(anna));
    assert_eq!(
        find_user_by_email(&pool, "nobody@example.se")
            .await
            .unwrap(),
        None
    );
}

#[tokio::test]
async fn second_user_needs_an_invitation() {
    let pool = db().await;
    register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    let err = register(&pool, "bo@example.se", "Bo", None, passkey("c2"), now())
        .await
        .unwrap_err();

    assert!(
        matches!(err, Error::Domain(DomainError::InvitationRequired)),
        "{err:?}"
    );
    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some("bogus"),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap_err();
    assert!(matches!(err, Error::InvitationNotFound), "{err:?}");
}

#[tokio::test]
async fn invited_user_registers_once_as_member() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let bo = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
    let again = register(
        &pool,
        "bo2@example.se",
        "Bo",
        Some(&token),
        passkey("c3"),
        now(),
    )
    .await
    .unwrap_err();

    assert_eq!(bo.role, Role::Member);
    assert!(
        matches!(again, Error::Domain(DomainError::InvitationAlreadyUsed)),
        "{again:?}"
    );
}

#[tokio::test]
async fn expired_invitation_is_rejected() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let later = now() + SignedDuration::from_hours(24 * 7);
    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        later,
    )
    .await
    .unwrap_err();

    assert!(
        matches!(err, Error::Domain(DomainError::InvitationExpired)),
        "{err:?}"
    );
}

#[tokio::test]
async fn invitation_tokens_are_stored_only_as_hashes() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let leaks: i64 = sqlx::query_scalar(
        "SELECT (SELECT COUNT(*) FROM events WHERE instr(payload, ?1) > 0)
              + (SELECT COUNT(*) FROM invitations WHERE instr(token_hash, ?1) > 0)",
    )
    .bind(&token)
    .fetch_one(&pool)
    .await
    .unwrap();
    assert_eq!(leaks, 0);
}

#[tokio::test]
async fn invitations_are_admin_only_and_unique_per_email() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let bo = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();

    let by_member = create_invitation(&pool, bo.id, "cecilia@example.se", now())
        .await
        .unwrap_err();
    let registered = create_invitation(&pool, anna.id, "BO@example.se", now())
        .await
        .unwrap_err();
    create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap();
    let pending = create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap_err();
    let after_expiry = now() + SignedDuration::from_hours(24 * 7);
    create_invitation(&pool, anna.id, "cecilia@example.se", after_expiry)
        .await
        .unwrap();

    assert!(
        matches!(by_member, Error::Domain(DomainError::NotAdmin)),
        "{by_member:?}"
    );
    assert!(matches!(registered, Error::AlreadyExists), "{registered:?}");
    assert!(matches!(pending, Error::AlreadyExists), "{pending:?}");
}

#[tokio::test]
async fn duplicate_email_is_rejected_without_writing_events() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    // A pending invitation for an email that registers in the meantime is the
    // only way to reach the users UNIQUE constraint through the public API.
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    sqlx::query("UPDATE users SET email = 'bo@example.se' WHERE user_id = ?")
        .bind(anna.id.to_string())
        .execute(&pool)
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let err = register(
        &pool,
        "Bo@Example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, Error::AlreadyExists), "{err:?}");
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn a_credential_cannot_belong_to_two_users() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let before = event_count(&pool).await;

    let err = register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c1"),
        now(),
    )
    .await
    .unwrap_err();

    assert!(matches!(err, Error::AlreadyExists), "{err:?}");
    assert_eq!(event_count(&pool).await, before);
}

#[tokio::test]
async fn users_can_add_passkeys_and_logins_update_them() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();

    add_passkey(&pool, anna.id, passkey("c2")).await.unwrap();
    let dup = add_passkey(&pool, anna.id, passkey("c2"))
        .await
        .unwrap_err();
    record_passkey_use(&pool, anna.id, "c2", json!({ "counter": 3 }))
        .await
        .unwrap();

    assert!(
        matches!(dup, Error::Domain(DomainError::DuplicatePasskey)),
        "{dup:?}"
    );
    let anna = get_user(&pool, anna.id).await.unwrap().unwrap();
    let ids: Vec<&str> = anna
        .passkeys
        .iter()
        .map(|p| p.credential_id.as_str())
        .collect();
    assert_eq!(ids, ["c1", "c2"]);
    assert_eq!(anna.passkeys[1].passkey, json!({ "counter": 3 }));
    let (stored, last_used): (String, Option<String>) =
        sqlx::query_as("SELECT passkey, last_used_at FROM passkeys WHERE credential_id = 'c2'")
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(stored, r#"{"counter":3}"#);
    assert!(last_used.is_some());
}

#[tokio::test]
async fn events_record_who_acted() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let actors: Vec<String> =
        sqlx::query_scalar("SELECT DISTINCT json_extract(metadata, '$.actor') FROM events")
            .fetch_all(&pool)
            .await
            .unwrap();
    assert_eq!(actors, [anna.id.to_string()]);
}

#[tokio::test]
async fn rebuilt_projections_equal_incremental_ones() {
    let pool = db().await;
    let anna = register(&pool, "anna@example.se", "Anna", None, passkey("c1"), now())
        .await
        .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    register(
        &pool,
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
    create_invitation(&pool, anna.id, "cecilia@example.se", now())
        .await
        .unwrap();
    record_passkey_use(&pool, anna.id, "c1", json!({ "counter": 1 }))
        .await
        .unwrap();
    let snapshot = || async {
        sqlx::query_scalar::<_, String>(
            "SELECT json_group_array(json_array(user_id, email, display_name, role, registered_at))
                 || (SELECT json_group_array(json_array(credential_id, user_id, name, passkey, added_at, last_used_at)) FROM (SELECT * FROM passkeys ORDER BY credential_id))
                 || (SELECT json_group_array(json_array(invitation_id, email, token_hash, created_by, expires_at, accepted_by)) FROM (SELECT * FROM invitations ORDER BY invitation_id))
             FROM (SELECT * FROM users ORDER BY user_id)",
        )
        .fetch_one(&pool)
        .await
        .unwrap()
    };
    let before = snapshot().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(snapshot().await, before);
    assert!(before.contains("cecilia@example.se"), "{before}");
}

#[tokio::test]
async fn concurrent_bootstrap_yields_exactly_one_admin() {
    let dir = tempfile::tempdir().unwrap();
    let url = format!("sqlite://{}", dir.path().join("doris.db").display());
    let pool = doris_eventstore::open(&url).await.unwrap();

    let tasks: Vec<_> = (0..4)
        .map(|i| {
            let pool = pool.clone();
            tokio::spawn(async move {
                register(
                    &pool,
                    &format!("u{i}@example.se"),
                    "U",
                    None,
                    passkey(&format!("c{i}")),
                    now(),
                )
                .await
            })
        })
        .collect();
    let mut admins = 0;
    for task in tasks {
        match task.await.unwrap() {
            Ok(user) => {
                assert_eq!(user.role, Role::Admin);
                admins += 1;
            }
            Err(err) => assert!(
                matches!(err, Error::Domain(DomainError::InvitationRequired)),
                "{err:?}"
            ),
        }
    }
    assert_eq!(admins, 1);
}
