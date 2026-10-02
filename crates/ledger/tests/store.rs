use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::DomainError;
use doris_ledger::{
    Error, add_account, list_accounts, rebuild_projections, rename_account, set_account_active,
};
use sqlx::SqlitePool;
use uuid::Uuid;

async fn db() -> SqlitePool {
    doris_eventstore::open("sqlite::memory:").await.unwrap()
}

/// A company whose first räkenskapsår is 2025, with `owner` as member.
async fn company(pool: &SqlitePool, owner: Uuid) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: "2025-01-01".parse().unwrap(),
            fiscal_year_end: "2025-12-31".parse().unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap()
}

async fn events_of(pool: &SqlitePool, prefix: &str) -> Vec<String> {
    sqlx::query_scalar(
        "SELECT event_type FROM events WHERE stream_id LIKE ? ORDER BY global_position",
    )
    .bind(format!("{prefix}%"))
    .fetch_all(pool)
    .await
    .unwrap()
}

async fn table(pool: &SqlitePool, sql: &'static str) -> Vec<String> {
    sqlx::query_scalar(sql).fetch_all(pool).await.unwrap()
}

#[tokio::test]
async fn a_new_company_lists_the_bas_selection_without_writing_anything() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let accounts = list_accounts(&pool, id, anna).await.unwrap();

    assert!(accounts.len() >= 150);
    assert!(accounts.iter().any(|a| a.number.get() == 1930 && a.active));
    assert!(events_of(&pool, "accounts-").await.is_empty());
}

#[tokio::test]
async fn the_first_change_seeds_the_chart_in_the_same_transaction() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    add_account(&pool, id, anna, 1931, "Sparkonto")
        .await
        .unwrap();
    rename_account(&pool, id, anna, 1931, "Sparkonto Handelsbanken")
        .await
        .unwrap();
    set_account_active(&pool, id, anna, 1910, false)
        .await
        .unwrap();
    set_account_active(&pool, id, anna, 1910, false)
        .await
        .unwrap();

    assert_eq!(
        events_of(&pool, "accounts-").await,
        [
            "ChartSeeded",
            "AccountAdded",
            "AccountRenamed",
            "AccountDeactivated"
        ]
    );
    let accounts = list_accounts(&pool, id, anna).await.unwrap();
    let get = |n: u16| accounts.iter().find(|a| a.number.get() == n).unwrap();
    assert_eq!(get(1931).name.as_str(), "Sparkonto Handelsbanken");
    assert!(!get(1910).active);
}

#[tokio::test]
async fn a_rejected_change_writes_nothing_not_even_the_seed() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let result = add_account(&pool, id, anna, 1930, "Bank").await;

    assert!(matches!(
        result,
        Err(Error::Domain(DomainError::AccountExists))
    ));
    assert!(events_of(&pool, "accounts-").await.is_empty());
}

#[tokio::test]
async fn invalid_input_is_refused() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    for (result, expected) in [
        (
            add_account(&pool, id, anna, 999, "X").await,
            DomainError::InvalidAccountNumber,
        ),
        (
            add_account(&pool, id, anna, 1931, " ").await,
            DomainError::InvalidAccountName,
        ),
        (
            rename_account(&pool, id, anna, 1999, "X").await,
            DomainError::AccountNotFound,
        ),
        (
            set_account_active(&pool, id, anna, 1999, false).await,
            DomainError::AccountNotFound,
        ),
    ] {
        assert!(
            matches!(result, Err(Error::Domain(e)) if e == expected),
            "{expected:?}"
        );
    }
}

#[tokio::test]
async fn non_members_get_not_found() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;

    assert!(matches!(
        list_accounts(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        add_account(&pool, id, bo, 1931, "X").await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_accounts(&pool, Uuid::new_v4(), anna).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_chart_projection_rebuilds_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    add_account(&pool, id, anna, 1931, "Sparkonto")
        .await
        .unwrap();
    set_account_active(&pool, id, anna, 1910, false)
        .await
        .unwrap();
    let sql =
        "SELECT company_id || number || name || active FROM accounts ORDER BY company_id, number";
    let before = table(&pool, sql).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(table(&pool, sql).await, before);
    assert!(!before.is_empty());
}
