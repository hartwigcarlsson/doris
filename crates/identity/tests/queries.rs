use doris_identity::domain::Passkey;
use doris_identity::{
    bootstrap_required, create_invitation, invitation_email, list_invitations, list_passkeys,
    record_passkey_use, register,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

fn passkey(id: &str, name: &str) -> Passkey {
    Passkey::new(id.into(), name, json!({})).unwrap()
}

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

#[tokio::test]
async fn bootstrap_is_required_until_the_first_user_registers() {
    let pool = db().await;
    assert!(bootstrap_required(&pool).await.unwrap());

    register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();

    assert!(!bootstrap_required(&pool).await.unwrap());
}

#[tokio::test]
async fn invitation_email_is_only_revealed_for_usable_invitations() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    let (_, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();

    let email = invitation_email(&pool, &token, now()).await.unwrap();
    let expired = invitation_email(&pool, &token, now() + SignedDuration::from_hours(24 * 7))
        .await
        .unwrap();
    let unknown = invitation_email(&pool, "nope", now()).await.unwrap();
    register(
        &pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2", "Telefon"),
        now(),
    )
    .await
    .unwrap();
    let used = invitation_email(&pool, &token, now()).await.unwrap();

    assert_eq!(email.unwrap().as_str(), "bo@example.se");
    assert_eq!(expired, None);
    assert_eq!(unknown, None);
    assert_eq!(used, None);
}

#[tokio::test]
async fn invitations_are_listed_with_their_status() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    let (bo_id, token) = create_invitation(&pool, anna.id, "bo@example.se", now())
        .await
        .unwrap();
    let later = now() + SignedDuration::from_hours(1);
    let (cecilia_id, _) = create_invitation(&pool, anna.id, "cecilia@example.se", later)
        .await
        .unwrap();
    register(
        &pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&token),
        passkey("c2", "Telefon"),
        now(),
    )
    .await
    .unwrap();

    let invitations = list_invitations(&pool).await.unwrap();

    let summary: Vec<(Uuid, &str, bool)> = invitations
        .iter()
        .map(|i| (i.id, i.email.as_str(), i.accepted))
        .collect();
    assert_eq!(
        summary,
        [
            (cecilia_id, "cecilia@example.se", false),
            (bo_id, "bo@example.se", true)
        ]
    );
    assert_eq!(
        invitations[1].expires_at,
        now() + SignedDuration::from_hours(24 * 7)
    );
}

#[tokio::test]
async fn passkeys_are_listed_per_user_with_last_use() {
    let pool = db().await;
    let anna = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1", "Laptop"),
        now(),
    )
    .await
    .unwrap();
    doris_identity::add_passkey(&pool, anna.id, passkey("c2", "Telefon"))
        .await
        .unwrap();
    record_passkey_use(&pool, anna.id, "c2", json!({ "counter": 1 }))
        .await
        .unwrap();

    let passkeys = list_passkeys(&pool, anna.id).await.unwrap();
    let nobody = list_passkeys(&pool, Uuid::new_v4()).await.unwrap();

    let names: Vec<&str> = passkeys.iter().map(|p| p.name.as_str()).collect();
    assert_eq!(names, ["Laptop", "Telefon"]);
    assert_eq!(passkeys[0].last_used_at, None);
    assert!(passkeys[1].last_used_at.is_some());
    assert!(nobody.is_empty());
}
