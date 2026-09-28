use doris_identity::domain::Passkey;
use doris_identity::{SESSION_TTL, create_session, end_session, register, session_user};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-09-28T10:00:00Z".parse().unwrap()
}

async fn db_with_user() -> (SqlitePool, Uuid) {
    let pool = doris_eventstore::open("sqlite::memory:").await.unwrap();
    let passkey = Passkey::new("c1".into(), "Laptop", json!({})).unwrap();
    let user = register(
        &pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey,
        now(),
    )
    .await
    .unwrap();
    (pool, user.id)
}

#[tokio::test]
async fn a_session_token_identifies_its_user() {
    let (pool, user_id) = db_with_user().await;

    let token = create_session(&pool, user_id, now()).await.unwrap();

    let user = session_user(&pool, &token, now()).await.unwrap().unwrap();
    assert_eq!(user.id, user_id);
    assert_eq!(
        session_user(&pool, "not-a-token", now()).await.unwrap(),
        None
    );
}

#[tokio::test]
async fn session_tokens_are_stored_only_as_hashes() {
    let (pool, user_id) = db_with_user().await;

    let token = create_session(&pool, user_id, now()).await.unwrap();

    let leaks: i64 =
        sqlx::query_scalar("SELECT COUNT(*) FROM sessions WHERE instr(token_hash, ?) > 0")
            .bind(&token)
            .fetch_one(&pool)
            .await
            .unwrap();
    assert_eq!(leaks, 0);
}

#[tokio::test]
async fn sessions_last_thirty_days() {
    let (pool, user_id) = db_with_user().await;
    let token = create_session(&pool, user_id, now()).await.unwrap();

    let almost = now() + SESSION_TTL - SignedDuration::from_secs(1);
    let expired = now() + SESSION_TTL;

    assert_eq!(SESSION_TTL, SignedDuration::from_hours(24 * 30));
    assert!(session_user(&pool, &token, almost).await.unwrap().is_some());
    assert_eq!(session_user(&pool, &token, expired).await.unwrap(), None);
}

#[tokio::test]
async fn ending_a_session_invalidates_only_that_token() {
    let (pool, user_id) = db_with_user().await;
    let laptop = create_session(&pool, user_id, now()).await.unwrap();
    let phone = create_session(&pool, user_id, now()).await.unwrap();

    end_session(&pool, &laptop).await.unwrap();

    assert_eq!(session_user(&pool, &laptop, now()).await.unwrap(), None);
    assert!(session_user(&pool, &phone, now()).await.unwrap().is_some());
}

#[tokio::test]
async fn creating_a_session_purges_expired_ones() {
    let (pool, user_id) = db_with_user().await;
    create_session(&pool, user_id, now()).await.unwrap();

    create_session(&pool, user_id, now() + SESSION_TTL)
        .await
        .unwrap();

    let count: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM sessions")
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(count, 1);
}
