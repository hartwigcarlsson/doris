use doris_identity::domain::{DomainError, Grant, Passkey, Scope};
use doris_identity::{
    Error, create_api_token, create_invitation, list_api_tokens, rebuild_projections, register,
    revoke_api_token, token_user, touch_api_token,
};
use jiff::{SignedDuration, Timestamp};
use serde_json::json;
use sqlx::SqlitePool;
use uuid::Uuid;

fn now() -> Timestamp {
    "2026-10-06T10:00:00Z".parse().unwrap()
}

const DAY: SignedDuration = SignedDuration::from_hours(24);

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// The first user (admin) and an invited member.
async fn admin_and_member(pool: &SqlitePool) -> (Uuid, Uuid) {
    let passkey = |id: &str| Passkey::new(id.into(), "Laptop", json!({ "cred": id })).unwrap();
    let admin = register(
        pool,
        Uuid::new_v4(),
        "anna@example.se",
        "Anna",
        None,
        passkey("c1"),
        now(),
    )
    .await
    .unwrap();
    let (_, invitation) = create_invitation(pool, admin.id, "bo@example.se", now())
        .await
        .unwrap();
    let member = register(
        pool,
        Uuid::new_v4(),
        "bo@example.se",
        "Bo",
        Some(&invitation),
        passkey("c2"),
        now(),
    )
    .await
    .unwrap();
    (admin.id, member.id)
}

fn grants(company: Uuid) -> Vec<Grant> {
    vec![Grant {
        company_id: company,
        scopes: vec![Scope::LedgerRead],
    }]
}

#[tokio::test]
async fn a_created_token_finds_its_owner_and_grants() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let company = Uuid::new_v4();

    let (id, secret) =
        create_api_token(&pool, bo, "Agent", now() + 30 * DAY, grants(company), now())
            .await
            .unwrap();

    assert!(
        secret.starts_with("doris_") && secret.len() > 40,
        "{secret}"
    );
    let (user, access) = token_user(&pool, &secret, now()).await.unwrap().unwrap();
    assert_eq!(user.id, bo);
    assert_eq!(access.token_id, id);
    assert_eq!(access.grants, grants(company));
    // Only the hash is stored.
    let stored: i64 = sqlx::query_scalar("SELECT COUNT(*) FROM events WHERE payload LIKE ?")
        .bind(format!("%{}%", &secret["doris_".len()..]))
        .fetch_one(&pool)
        .await
        .unwrap();
    assert_eq!(stored, 0);
}

#[tokio::test]
async fn unknown_expired_and_revoked_tokens_find_no_one() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (id, secret) = create_api_token(
        &pool,
        bo,
        "Agent",
        now() + DAY,
        grants(Uuid::new_v4()),
        now(),
    )
    .await
    .unwrap();

    assert!(
        token_user(&pool, "doris_nope", now())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        token_user(&pool, &secret["doris_".len()..], now())
            .await
            .unwrap()
            .is_none()
    );
    assert!(
        token_user(&pool, &secret, now() + DAY)
            .await
            .unwrap()
            .is_none()
    );

    revoke_api_token(&pool, bo, id).await.unwrap();
    assert!(token_user(&pool, &secret, now()).await.unwrap().is_none());
}

#[tokio::test]
async fn tokens_are_listed_newest_first_with_their_state() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let company = Uuid::new_v4();
    let (first, _) = create_api_token(&pool, bo, "Första", now() + DAY, grants(company), now())
        .await
        .unwrap();
    let (second, _) = create_api_token(&pool, bo, "Andra", now() + 2 * DAY, grants(company), now())
        .await
        .unwrap();
    revoke_api_token(&pool, bo, first).await.unwrap();
    touch_api_token(&pool, second, now()).await.unwrap();

    let list = list_api_tokens(&pool, bo).await.unwrap();

    assert_eq!(
        list.iter().map(|t| t.id).collect::<Vec<_>>(),
        [second, first]
    );
    assert_eq!(list[0].name, "Andra");
    assert_eq!(list[0].grants, grants(company));
    assert_eq!(list[0].expires_at, now() + 2 * DAY);
    assert_eq!(list[0].last_used_at, Some(now()));
    assert_eq!(list[0].revoked_at, None);
    assert_eq!(list[1].last_used_at, None);
    assert!(list[1].revoked_at.is_some());
}

#[tokio::test]
async fn last_use_is_written_at_most_once_an_hour() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (id, _) = create_api_token(
        &pool,
        bo,
        "Agent",
        now() + DAY,
        grants(Uuid::new_v4()),
        now(),
    )
    .await
    .unwrap();
    let last_used = || async { list_api_tokens(&pool, bo).await.unwrap()[0].last_used_at };

    touch_api_token(&pool, id, now()).await.unwrap();
    touch_api_token(&pool, id, now() + SignedDuration::from_mins(59))
        .await
        .unwrap();
    assert_eq!(last_used().await, Some(now()));

    touch_api_token(&pool, id, now() + SignedDuration::from_mins(60))
        .await
        .unwrap();
    assert_eq!(
        last_used().await,
        Some(now() + SignedDuration::from_mins(60))
    );
}

#[tokio::test]
async fn an_admin_revokes_any_token_and_others_see_none() {
    let pool = db().await;
    let (anna, bo) = admin_and_member(&pool).await;
    let (annas, _) = create_api_token(
        &pool,
        anna,
        "Anna",
        now() + DAY,
        grants(Uuid::new_v4()),
        now(),
    )
    .await
    .unwrap();
    let (bos, _) = create_api_token(&pool, bo, "Bo", now() + DAY, grants(Uuid::new_v4()), now())
        .await
        .unwrap();

    assert!(matches!(
        revoke_api_token(&pool, bo, annas).await,
        Err(Error::Domain(DomainError::NotTokenOwner))
    ));
    assert!(matches!(
        revoke_api_token(&pool, bo, Uuid::new_v4()).await,
        Err(Error::ApiTokenNotFound)
    ));
    revoke_api_token(&pool, anna, bos).await.unwrap();
    // Again, as from a second tab: nothing happens.
    revoke_api_token(&pool, anna, bos).await.unwrap();
    assert!(
        list_api_tokens(&pool, bo).await.unwrap()[0]
            .revoked_at
            .is_some()
    );
}

type Row = (
    String,
    String,
    String,
    String,
    String,
    String,
    i64,
    Option<String>,
);

#[tokio::test]
async fn api_tokens_rebuild_from_the_log() {
    let pool = db().await;
    let (_, bo) = admin_and_member(&pool).await;
    let (first, _) = create_api_token(
        &pool,
        bo,
        "Första",
        now() + DAY,
        grants(Uuid::new_v4()),
        now(),
    )
    .await
    .unwrap();
    create_api_token(
        &pool,
        bo,
        "Andra",
        now() + DAY,
        grants(Uuid::new_v4()),
        now(),
    )
    .await
    .unwrap();
    revoke_api_token(&pool, bo, first).await.unwrap();
    let snapshot = || async {
        let rows: Vec<Row> = sqlx::query_as("SELECT * FROM api_tokens ORDER BY token_id")
            .fetch_all(&pool)
            .await
            .unwrap();
        rows
    };
    let before = snapshot().await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(snapshot().await, before);
    assert_eq!(before.len(), 2);
}
