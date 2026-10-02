use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{DomainError, RecordVoucher, VoucherLine};
use doris_ledger::{
    Error, VoucherRef, add_account, correct_voucher, list_accounts, list_fiscal_years,
    list_vouchers, rebuild_projections, record_voucher, record_voucher_in, rename_account,
    set_account_active,
};
use jiff::civil::Date;
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

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

const TODAY: &str = "2026-10-02";

fn sale(date: &str, ore: i64) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: "Försäljning".into(),
        lines: vec![
            VoucherLine::new(1930, ore, 0).unwrap(),
            VoucherLine::new(3001, 0, ore).unwrap(),
        ],
    }
}

#[tokio::test]
async fn vouchers_are_numbered_per_fiscal_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    let a = record_voucher(&pool, id, anna, sale("2025-12-31", 100), today)
        .await
        .unwrap();
    let b = record_voucher(&pool, id, anna, sale("2026-01-01", 100), today)
        .await
        .unwrap();
    let c = record_voucher(&pool, id, anna, sale("2025-02-01", 100), today)
        .await
        .unwrap();

    assert_eq!(
        a,
        VoucherRef {
            fiscal_year_start: d("2025-01-01"),
            number: 1
        }
    );
    assert_eq!(
        b,
        VoucherRef {
            fiscal_year_start: d("2026-01-01"),
            number: 1
        }
    );
    assert_eq!(
        c,
        VoucherRef {
            fiscal_year_start: d("2025-01-01"),
            number: 2
        }
    );
    let in_2025 = list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    assert_eq!(in_2025.iter().map(|v| v.number).collect::<Vec<_>>(), [1, 2]);
    assert_eq!(in_2025[0].date, d("2025-12-31"));
    assert_eq!(in_2025[0].lines, sale("2025-12-31", 100).lines);
}

#[tokio::test]
async fn a_rejected_voucher_uses_no_number_and_writes_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let mut unbalanced = sale("2025-03-01", 100);
    unbalanced.lines[1].credit = 99;

    let result = record_voucher(&pool, id, anna, unbalanced, today).await;
    let next = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();

    assert!(matches!(
        result,
        Err(Error::Domain(DomainError::VoucherUnbalanced))
    ));
    assert_eq!(next.number, 1);
    assert_eq!(events_of(&pool, "ledger-").await, ["VoucherRecorded"]);
}

#[tokio::test]
async fn a_rolled_back_transaction_uses_no_number() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    let mut tx = doris_eventstore::begin(&pool).await.unwrap();
    let abandoned = record_voucher_in(&mut tx, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    tx.rollback().await.unwrap();
    let kept = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();

    assert_eq!(abandoned.number, 1);
    assert_eq!(kept.number, 1);
    assert!(events_of(&pool, "accounts-").await == ["ChartSeeded"]);
}

#[tokio::test]
async fn the_first_voucher_seeds_the_chart_it_is_checked_against() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY))
        .await
        .unwrap();

    assert_eq!(events_of(&pool, "accounts-").await, ["ChartSeeded"]);
}

#[tokio::test]
async fn a_correction_is_listed_with_both_links() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let original = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();

    let correction = correct_voucher(
        &pool,
        id,
        anna,
        original.fiscal_year_start,
        1,
        d("2025-12-31"),
        today,
    )
    .await
    .unwrap();
    let again = correct_voucher(
        &pool,
        id,
        anna,
        original.fiscal_year_start,
        1,
        d("2025-12-31"),
        today,
    )
    .await;

    assert_eq!(
        correction,
        VoucherRef {
            fiscal_year_start: d("2025-01-01"),
            number: 2
        }
    );
    assert!(matches!(
        again,
        Err(Error::Domain(DomainError::AlreadyCorrected))
    ));
    let listed = list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    assert_eq!(
        (listed[0].corrects, listed[0].corrected_by),
        (None, Some(2))
    );
    assert_eq!(
        (listed[1].corrects, listed[1].corrected_by),
        (Some(1), None)
    );
    assert_eq!(listed[1].text, "Rättelse av ver 1");
}

#[tokio::test]
async fn correcting_in_a_year_that_is_not_a_fiscal_year_start_finds_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();

    for start in ["2025-02-01", "2024-01-01", "2027-01-01"] {
        let result = correct_voucher(&pool, id, anna, d(start), 1, d("2025-03-02"), today).await;
        assert!(
            matches!(result, Err(Error::Domain(DomainError::VoucherNotFound))),
            "{start}"
        );
    }
}

#[tokio::test]
async fn correcting_in_a_far_future_year_finds_nothing_and_does_not_panic() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    // A broken räkenskapsår: stepping years towards 9999-12-31 overflows.
    let id = doris_company::register_company(
        &pool,
        anna,
        NewCompany {
            org_nr: "556016-0680",
            name: "Exempel AB",
            legal_form: LegalForm::Aktiebolag,
            street: "",
            postal_code: "",
            city: "",
            fiscal_year_start: "2025-05-01".parse().unwrap(),
            fiscal_year_end: "2026-04-30".parse().unwrap(),
            accounting_method: AccountingMethod::Invoice,
        },
    )
    .await
    .unwrap();

    for start in ["9999-05-01", "9999-12-31"] {
        let result = correct_voucher(&pool, id, anna, d(start), 1, d("2025-06-01"), d(TODAY)).await;
        assert!(
            matches!(result, Err(Error::Domain(DomainError::VoucherNotFound))),
            "{start}"
        );
    }
}

#[tokio::test]
async fn fiscal_years_run_from_the_first_to_the_current_newest_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let years = list_fiscal_years(&pool, id, anna, d(TODAY)).await.unwrap();
    let before_start = list_fiscal_years(&pool, id, anna, d("2024-06-01"))
        .await
        .unwrap();

    assert_eq!(
        years.iter().map(|y| y.start).collect::<Vec<_>>(),
        [d("2026-01-01"), d("2025-01-01")]
    );
    assert_eq!(
        before_start.iter().map(|y| y.start).collect::<Vec<_>>(),
        [d("2025-01-01")]
    );
}

#[tokio::test]
async fn non_members_cannot_book_correct_or_read() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();

    assert!(matches!(
        record_voucher(&pool, id, bo, sale("2025-03-01", 1), today).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        correct_voucher(&pool, id, bo, d("2025-01-01"), 1, d("2025-03-02"), today).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_vouchers(&pool, id, bo, d("2025-01-01")).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_fiscal_years(&pool, id, bo, today).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_database_refuses_a_gap_or_a_duplicate_in_the_projection() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY))
        .await
        .unwrap();
    let insert = |number: i64| {
        sqlx::query(
            "INSERT INTO vouchers (company_id, fiscal_year_start, number, date, text, recorded_at, recorded_by)
             VALUES (?, '2025-01-01', ?, '2025-03-01', 'x', 'now', 'test')",
        )
        .bind(id.to_string())
        .bind(number)
        .execute(&pool)
    };

    assert!(insert(3).await.is_err(), "a gap");
    assert!(insert(1).await.is_err(), "a duplicate");
    assert!(insert(2).await.is_ok(), "the next number");
}

#[tokio::test]
async fn the_voucher_projections_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2026-03-01", 250), today)
        .await
        .unwrap();
    correct_voucher(&pool, id, anna, d("2025-01-01"), 1, d("2025-03-02"), today)
        .await
        .unwrap();
    let before_2025 = list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    let before_2026 = list_vouchers(&pool, id, anna, d("2026-01-01"))
        .await
        .unwrap();
    let lines_sql = "SELECT company_id || fiscal_year_start || number || line_no || account || debit || credit FROM voucher_lines ORDER BY 1";
    let audit_sql = "SELECT number || recorded_at || recorded_by FROM vouchers ORDER BY 1";
    let (lines, audit) = (table(&pool, lines_sql).await, table(&pool, audit_sql).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(
        list_vouchers(&pool, id, anna, d("2025-01-01"))
            .await
            .unwrap(),
        before_2025
    );
    assert_eq!(
        list_vouchers(&pool, id, anna, d("2026-01-01"))
            .await
            .unwrap(),
        before_2026
    );
    assert_eq!(table(&pool, lines_sql).await, lines);
    assert_eq!(table(&pool, audit_sql).await, audit);
    assert!(audit.iter().all(|row| row.ends_with(&anna.to_string())));
}
