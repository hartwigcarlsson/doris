use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_ledger::domain::{ContentType, DomainError, RecordVoucher, TrialBalanceRow, VoucherLine};
use doris_ledger::statements::StatementLine;
use doris_ledger::{
    Error, NewAttachment, VoucherRef, account_ledger, add_account, add_attachment,
    close_fiscal_year, correct_voucher, financial_statements, get_attachment, list_accounts,
    list_fiscal_years, list_vouchers, opening_balances, rebuild_projections, record_voucher,
    record_voucher_in, record_voucher_with_attachments, rename_account, reopen_fiscal_year,
    set_account_active, set_opening_balances, trial_balance,
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
        years
            .iter()
            .map(|y| y.fiscal_year.start)
            .collect::<Vec<_>>(),
        [d("2026-01-01"), d("2025-01-01")]
    );
    assert_eq!(
        before_start
            .iter()
            .map(|y| y.fiscal_year.start)
            .collect::<Vec<_>>(),
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

/// Another company of `owner`'s, also with first räkenskapsår 2025.
async fn second_company(pool: &SqlitePool, owner: Uuid) -> Uuid {
    doris_company::register_company(
        pool,
        owner,
        NewCompany {
            org_nr: "556036-0793",
            name: "Bolaget AB",
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

fn booking(date: &str, text: &str, lines: &[(u32, i64, i64)]) -> RecordVoucher {
    RecordVoucher {
        date: d(date),
        text: text.into(),
        lines: lines
            .iter()
            .map(|&(account, debit, credit)| VoucherLine::new(account, debit, credit).unwrap())
            .collect(),
    }
}

#[tokio::test]
async fn the_trial_balance_sums_each_account_in_one_fiscal_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let other = second_company(&pool, anna).await;
    let today = d(TODAY);
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-04-01", 250), today)
        .await
        .unwrap();
    record_voucher(
        &pool,
        id,
        anna,
        booking("2025-05-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        today,
    )
    .await
    .unwrap();
    // Reverses voucher 1: both it and the correction are counted.
    correct_voucher(&pool, id, anna, d("2025-01-01"), 1, d("2025-06-01"), today)
        .await
        .unwrap();
    // Another fiscal year, and another company: neither is counted.
    record_voucher(&pool, id, anna, sale("2026-01-10", 999), today)
        .await
        .unwrap();
    record_voucher(&pool, other, anna, sale("2025-03-01", 777), today)
        .await
        .unwrap();

    let rows = trial_balance(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();

    assert_eq!(
        rows,
        vec![
            TrialBalanceRow {
                account: 1930,
                name: "Företagskonto/checkkonto/affärskonto".into(),
                opening: 0,
                debit: 350,
                credit: 400,
            },
            TrialBalanceRow {
                account: 3001,
                name: "Försäljning inom Sverige, 25 % moms".into(),
                opening: 0,
                debit: 100,
                credit: 350,
            },
            TrialBalanceRow {
                account: 5010,
                name: "Lokalhyra".into(),
                opening: 0,
                debit: 300,
                credit: 0,
            },
        ]
    );
    assert_eq!(rows.iter().map(|r| r.debit - r.credit).sum::<i64>(), 0);
    assert!(
        trial_balance(&pool, id, anna, d("2024-01-01"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_accounts_ledger_runs_in_date_order_with_its_balance() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    // Voucher 1 is dated after vouchers 2 and 3.
    for cmd in [
        sale("2025-04-01", 250),
        sale("2025-03-01", 100),
        booking("2025-03-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        booking("2025-05-01", "Omföring", &[(1930, 40, 0), (1930, 0, 40)]),
    ] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    let entries = account_ledger(&pool, id, anna, d("2025-01-01"), 1930)
        .await
        .unwrap()
        .entries;

    assert_eq!(
        entries
            .iter()
            .map(|e| (e.date, e.number, e.debit, e.credit, e.balance))
            .collect::<Vec<_>>(),
        vec![
            (d("2025-03-01"), 2, 100, 0, 100),
            (d("2025-03-01"), 3, 0, 300, -200),
            (d("2025-04-01"), 1, 250, 0, 50),
            (d("2025-05-01"), 4, 40, 0, 90),
            (d("2025-05-01"), 4, 0, 40, 50),
        ]
    );
    assert_eq!(entries[1].text, "Hyra");
    assert!(
        account_ledger(&pool, id, anna, d("2025-01-01"), 1931)
            .await
            .unwrap()
            .entries
            .is_empty()
    );
    assert!(matches!(
        account_ledger(&pool, id, anna, d("2025-01-01"), 999).await,
        Err(Error::Domain(DomainError::InvalidAccountNumber))
    ));
}

#[tokio::test]
async fn non_members_cannot_read_the_reports() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), d(TODAY))
        .await
        .unwrap();

    assert!(matches!(
        trial_balance(&pool, id, bo, d("2025-01-01")).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        account_ledger(&pool, id, bo, d("2025-01-01"), 1930).await,
        Err(Error::NotFound)
    ));
    // Membership is checked before the account number.
    assert!(matches!(
        account_ledger(&pool, id, bo, d("2025-01-01"), 999).await,
        Err(Error::NotFound)
    ));
}

fn ib(lines: &[(u32, i64, i64)]) -> Vec<VoucherLine> {
    lines
        .iter()
        .map(|&(account, debit, credit)| VoucherLine::new(account, debit, credit).unwrap())
        .collect()
}

#[tokio::test]
async fn opening_balances_are_set_and_replaced_in_the_first_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    set_opening_balances(&pool, id, anna, ib(&[(1930, 5_000, 0), (2081, 0, 5_000)]))
        .await
        .unwrap();

    assert_eq!(
        events_of(&pool, "ledger-").await,
        ["OpeningBalancesSet", "OpeningBalancesSet"]
    );
    assert_eq!(
        table(
            &pool,
            "SELECT account || ':' || debit || ':' || credit FROM opening_balances ORDER BY account"
        )
        .await,
        ["1930:5000:0", "2081:0:5000"]
    );
    assert!(matches!(
        set_opening_balances(&pool, id, anna, ib(&[(1930, 1, 0)])).await,
        Err(Error::Domain(DomainError::OpeningBalancesUnbalanced))
    ));
}

#[tokio::test]
async fn closing_books_the_result_and_locks_the_year_until_it_is_reopened() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();

    assert_eq!(
        close_fiscal_year(&pool, id, anna, start, today)
            .await
            .unwrap(),
        Some(2)
    );
    assert!(matches!(
        record_voucher(&pool, id, anna, sale("2025-03-02", 1), today).await,
        Err(Error::Domain(DomainError::FiscalYearClosed))
    ));
    assert!(matches!(
        correct_voucher(&pool, id, anna, start, 1, d("2025-03-02"), today).await,
        Err(Error::Domain(DomainError::FiscalYearClosed))
    ));
    assert_eq!(
        table(
            &pool,
            "SELECT fiscal_year_start || ':' || closed_by FROM closed_fiscal_years"
        )
        .await,
        [format!("2025-01-01:{anna}")]
    );

    assert_eq!(
        reopen_fiscal_year(&pool, id, anna, start, "Glömd faktura", today)
            .await
            .unwrap(),
        Some(3)
    );
    assert!(
        table(&pool, "SELECT fiscal_year_start FROM closed_fiscal_years")
            .await
            .is_empty()
    );
    let vouchers = list_vouchers(&pool, id, anna, start).await.unwrap();
    assert_eq!(
        vouchers
            .iter()
            .map(|v| (v.number, v.text.as_str(), v.corrects))
            .collect::<Vec<_>>(),
        vec![
            (1, "Försäljning", None),
            (2, "Årets resultat", None),
            (3, "Rättelse av ver 2", Some(2)),
        ]
    );
    record_voucher(&pool, id, anna, sale("2025-03-02", 1), today)
        .await
        .unwrap();
}

#[tokio::test]
async fn years_close_oldest_first_and_reopen_newest_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    // 2025, 2026 and 2027 have ended; 2028 has not.
    let today = d("2028-02-01");
    let close = |start: &str| close_fiscal_year(&pool, id, anna, d(start), today);
    let reopen = |start: &str| reopen_fiscal_year(&pool, id, anna, d(start), "Fel", today);

    assert!(matches!(
        close("2026-01-01").await,
        Err(Error::Domain(DomainError::PreviousFiscalYearOpen))
    ));
    close("2025-01-01").await.unwrap();
    close("2026-01-01").await.unwrap();
    assert!(matches!(
        close("2028-01-01").await,
        Err(Error::Domain(DomainError::FiscalYearNotEnded))
    ));
    assert!(matches!(
        reopen("2025-01-01").await,
        Err(Error::Domain(DomainError::LaterFiscalYearClosed))
    ));
    assert!(matches!(
        reopen("2027-01-01").await,
        Err(Error::Domain(DomainError::FiscalYearOpen))
    ));
    reopen("2026-01-01").await.unwrap();
    reopen("2025-01-01").await.unwrap();
}

#[tokio::test]
async fn a_date_that_does_not_start_a_fiscal_year_is_not_found() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    // Mid-year, before the first year, and after today.
    for start in ["2025-02-01", "2024-01-01", "2027-01-01", "9999-01-01"] {
        assert!(
            matches!(
                close_fiscal_year(&pool, id, anna, d(start), today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "close {start}"
        );
        assert!(
            matches!(
                reopen_fiscal_year(&pool, id, anna, d(start), "Fel", today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "reopen {start}"
        );
    }
}

#[tokio::test]
async fn non_members_cannot_set_balances_close_or_reopen() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));

    assert!(matches!(
        set_opening_balances(&pool, id, bo, ib(&[(1930, 1, 0), (2081, 0, 1)])).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        close_fiscal_year(&pool, id, bo, start, today).await,
        Err(Error::NotFound)
    ));
    close_fiscal_year(&pool, id, anna, start, today)
        .await
        .unwrap();
    assert!(matches!(
        reopen_fiscal_year(&pool, id, bo, start, "Fel", today).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn opening_balances_and_closings_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();
    close_fiscal_year(&pool, id, anna, start, today)
        .await
        .unwrap();
    reopen_fiscal_year(&pool, id, anna, start, "Fel", today)
        .await
        .unwrap();
    close_fiscal_year(&pool, id, anna, start, today)
        .await
        .unwrap();
    let ib_sql = "SELECT company_id || account || ':' || debit || ':' || credit FROM opening_balances ORDER BY 1";
    let closed_sql = "SELECT company_id || fiscal_year_start || closed_at || closed_by FROM closed_fiscal_years ORDER BY 1";
    let lines_sql = "SELECT company_id || fiscal_year_start || number || line_no || account || debit || credit FROM voucher_lines ORDER BY 1";
    let (ib_rows, closed, lines) = (
        table(&pool, ib_sql).await,
        table(&pool, closed_sql).await,
        table(&pool, lines_sql).await,
    );

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(ib_rows.len(), 2);
    assert_eq!(closed.len(), 1);
    assert_eq!(table(&pool, ib_sql).await, ib_rows);
    assert_eq!(table(&pool, closed_sql).await, closed);
    assert_eq!(table(&pool, lines_sql).await, lines);
}

/// (account, opening, debit, credit) per row.
fn figures(rows: &[TrialBalanceRow]) -> Vec<(u32, i64, i64, i64)> {
    rows.iter()
        .map(|r| (r.account, r.opening, r.debit, r.credit))
        .collect()
}

#[tokio::test]
async fn later_years_open_with_the_balance_sheet_carried_forward() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let (y2025, y2026) = (d("2025-01-01"), d("2026-01-01"));
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    for cmd in [
        sale("2025-03-01", 1_000),
        booking("2025-04-01", "Hyra", &[(5010, 300, 0), (1930, 0, 300)]),
        sale("2026-02-01", 50),
    ] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    assert_eq!(
        figures(&trial_balance(&pool, id, anna, y2025).await.unwrap()),
        vec![
            (1930, 10_000, 1_000, 300),
            (2081, -10_000, 0, 0),
            (3001, 0, 0, 1_000),
            (5010, 0, 300, 0),
        ]
    );
    // 2025 is open, so its result (a 700 profit) isn't on 2099 yet and
    // 2026 opens off by exactly that.
    let open = trial_balance(&pool, id, anna, y2026).await.unwrap();
    assert_eq!(
        figures(&open),
        vec![
            (1930, 10_700, 50, 0),
            (2081, -10_000, 0, 0),
            (3001, 0, 0, 50)
        ]
    );
    assert_eq!(open.iter().map(|r| r.opening).sum::<i64>(), 700);

    close_fiscal_year(&pool, id, anna, y2025, today)
        .await
        .unwrap();
    let closed = trial_balance(&pool, id, anna, y2026).await.unwrap();
    assert_eq!(
        figures(&closed),
        vec![
            (1930, 10_700, 50, 0),
            (2081, -10_000, 0, 0),
            (2099, -700, 0, 0),
            (3001, 0, 0, 50),
        ]
    );
    assert_eq!(closed.iter().map(|r| r.opening).sum::<i64>(), 0);
    assert_eq!(closed[2].name, "Årets resultat");

    // Reopening reverses the result voucher: 2099 nets to 0 and drops out.
    reopen_fiscal_year(&pool, id, anna, y2025, "Glömd faktura", today)
        .await
        .unwrap();
    assert_eq!(
        figures(&trial_balance(&pool, id, anna, y2026).await.unwrap()),
        figures(&open)
    );
    // A year before the first has nothing, not even the opening balances.
    assert!(
        trial_balance(&pool, id, anna, d("2024-01-01"))
            .await
            .unwrap()
            .is_empty()
    );
}

#[tokio::test]
async fn an_accounts_ledger_starts_from_its_opening_balance() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let y2026 = d("2026-01-01");
    set_opening_balances(&pool, id, anna, ib(&[(1930, 10_000, 0), (2081, 0, 10_000)]))
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2025-03-01", 1_000), today)
        .await
        .unwrap();
    record_voucher(&pool, id, anna, sale("2026-02-01", 50), today)
        .await
        .unwrap();

    let bank = account_ledger(&pool, id, anna, y2026, 1930).await.unwrap();
    assert_eq!(bank.opening, 11_000);
    assert_eq!(
        bank.entries.iter().map(|e| e.balance).collect::<Vec<_>>(),
        [11_050]
    );
    // Income accounts start every year at 0.
    assert_eq!(
        account_ledger(&pool, id, anna, y2026, 3001)
            .await
            .unwrap()
            .opening,
        0
    );
    // An account with only an opening balance has no entries.
    let capital = account_ledger(&pool, id, anna, y2026, 2081).await.unwrap();
    assert_eq!((capital.opening, capital.entries.len()), (-10_000, 0));
    let first_year = account_ledger(&pool, id, anna, d("2025-01-01"), 2081)
        .await
        .unwrap();
    assert_eq!(first_year.opening, -10_000);
}

#[tokio::test]
async fn fiscal_years_say_whether_they_are_closed() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let states = || async {
        list_fiscal_years(&pool, id, anna, today)
            .await
            .unwrap()
            .iter()
            .map(|y| (y.fiscal_year.start, y.closed))
            .collect::<Vec<_>>()
    };

    close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    assert_eq!(
        states().await,
        [(d("2026-01-01"), false), (d("2025-01-01"), true)]
    );

    reopen_fiscal_year(&pool, id, anna, d("2025-01-01"), "Fel", today)
        .await
        .unwrap();
    assert_eq!(
        states().await,
        [(d("2026-01-01"), false), (d("2025-01-01"), false)]
    );
}

#[tokio::test]
async fn the_first_years_opening_balances_are_read_back_by_account() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    assert!(opening_balances(&pool, id, anna).await.unwrap().is_empty());

    set_opening_balances(&pool, id, anna, ib(&[(2081, 0, 10_000), (1930, 10_000, 0)]))
        .await
        .unwrap();

    assert_eq!(
        opening_balances(&pool, id, anna).await.unwrap(),
        ib(&[(1930, 10_000, 0), (2081, 0, 10_000)])
    );
    assert!(matches!(
        opening_balances(&pool, id, bo).await,
        Err(Error::NotFound)
    ));
}

/// A small PDF: the header plus `body`.
fn file(name: &str, body: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: format!("%PDF-1.7\n{body}").into_bytes(),
    }
}

fn png(name: &str) -> NewAttachment {
    NewAttachment {
        file_name: name.into(),
        data: b"\x89PNG\r\n\x1a\nbild".to_vec(),
    }
}

#[tokio::test]
async fn a_voucher_is_booked_with_its_attachments_in_one_transaction() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let booked = record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("Kvitto åäö 🧾.pdf", "a"), png("foto.png")],
        d(TODAY),
    )
    .await
    .unwrap();

    assert_eq!(booked.number, 1);
    assert_eq!(
        events_of(&pool, "ledger-").await,
        ["VoucherRecorded", "AttachmentAdded", "AttachmentAdded"]
    );
    let vouchers = list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    let listed: Vec<_> = vouchers[0]
        .attachments
        .iter()
        .map(|a| (a.file_name.as_str(), a.content_type, a.size))
        .collect();
    assert_eq!(
        listed,
        [
            ("Kvitto åäö 🧾.pdf", ContentType::Pdf, 10),
            ("foto.png", ContentType::Png, 12)
        ]
    );
    assert_eq!(vouchers[0].attachments[0].sha256.len(), 64);
    assert_eq!(
        table(
            &pool,
            "SELECT position || ':' || added_by FROM voucher_attachments ORDER BY position"
        )
        .await,
        [format!("1:{anna}"), format!("2:{anna}")]
    );
}

#[tokio::test]
async fn a_file_is_stored_once_however_often_it_is_attached() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("kvitto.pdf", "a")],
        today,
    )
    .await
    .unwrap();
    record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-02", 100),
        vec![file("kopia.pdf", "a")],
        today,
    )
    .await
    .unwrap();

    assert_eq!(
        table(&pool, "SELECT sha256 FROM attachment_files")
            .await
            .len(),
        1
    );
    assert_eq!(
        table(&pool, "SELECT sha256 FROM voucher_attachments")
            .await
            .len(),
        2
    );
}

#[tokio::test]
async fn a_rejected_booking_keeps_neither_the_voucher_nor_its_files() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    let unbalanced = RecordVoucher {
        lines: vec![
            VoucherLine::new(1930, 100, 0).unwrap(),
            VoucherLine::new(3001, 0, 99).unwrap(),
        ],
        ..sale("2025-03-01", 100)
    };
    let gif = NewAttachment {
        file_name: "bild.gif".into(),
        data: b"GIF89a".to_vec(),
    };

    assert!(matches!(
        record_voucher_with_attachments(
            &pool,
            id,
            anna,
            unbalanced,
            vec![file("kvitto.pdf", "a")],
            today
        )
        .await,
        Err(Error::Domain(DomainError::VoucherUnbalanced))
    ));
    assert!(matches!(
        record_voucher_with_attachments(
            &pool,
            id,
            anna,
            sale("2025-03-01", 100),
            vec![file("kvitto.pdf", "a"), gif],
            today
        )
        .await,
        Err(Error::Domain(DomainError::UnsupportedAttachmentType))
    ));
    assert!(matches!(
        record_voucher_with_attachments(
            &pool,
            id,
            anna,
            sale("2025-03-01", 100),
            vec![file("a.pdf", "x"), file("b.pdf", "x")],
            today
        )
        .await,
        Err(Error::Domain(DomainError::DuplicateAttachment))
    ));

    assert!(events_of(&pool, "ledger-").await.is_empty());
    assert!(
        table(&pool, "SELECT sha256 FROM attachment_files")
            .await
            .is_empty()
    );
    let booked = record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    assert_eq!(booked.number, 1);
}

#[tokio::test]
async fn attachment_files_are_append_only() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("kvitto.pdf", "a")],
        d(TODAY),
    )
    .await
    .unwrap();

    for sql in [
        "UPDATE attachment_files SET size = 0",
        "DELETE FROM attachment_files",
    ] {
        let err = sqlx::query(sql).execute(&pool).await.unwrap_err();
        assert!(err.to_string().contains("append-only"), "{sql}: {err}");
    }
}

#[tokio::test]
async fn an_attachment_is_added_later_even_in_a_closed_year() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher(&pool, id, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    close_fiscal_year(&pool, id, anna, start, today)
        .await
        .unwrap();

    let added = add_attachment(&pool, id, anna, start, 1, file("faktura.pdf", "f"), today)
        .await
        .unwrap();

    assert_eq!(added.file_name.as_str(), "faktura.pdf");
    let vouchers = list_vouchers(&pool, id, anna, start).await.unwrap();
    assert_eq!(vouchers[0].attachments, vec![added]);
    for (fy_start, number) in [(start, 99), (d("2025-02-01"), 1), (d("2999-01-01"), 1)] {
        assert!(
            matches!(
                add_attachment(&pool, id, anna, fy_start, number, file("x.pdf", "x"), today).await,
                Err(Error::Domain(DomainError::VoucherNotFound))
            ),
            "{fy_start} {number}"
        );
    }
    assert!(matches!(
        add_attachment(
            &pool,
            id,
            Uuid::new_v4(),
            start,
            1,
            file("x.pdf", "x"),
            today
        )
        .await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn attachments_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("a.pdf", "a"), file("b.pdf", "b")],
        today,
    )
    .await
    .unwrap();
    add_attachment(&pool, id, anna, start, 1, file("c.pdf", "c"), today)
        .await
        .unwrap();
    let sql = "SELECT company_id || fiscal_year_start || number || position || sha256 || file_name
               || content_type || size || added_at || added_by FROM voucher_attachments ORDER BY 1";
    let before = table(&pool, sql).await;
    let listed = list_vouchers(&pool, id, anna, start).await.unwrap();

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(before.len(), 3);
    assert_eq!(table(&pool, sql).await, before);
    assert_eq!(list_vouchers(&pool, id, anna, start).await.unwrap(), listed);
}

#[tokio::test]
async fn an_attachment_is_read_only_through_the_companys_own_voucher() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let other = second_company(&pool, anna).await;
    let (start, today) = (d("2025-01-01"), d(TODAY));
    record_voucher_with_attachments(
        &pool,
        id,
        anna,
        sale("2025-03-01", 100),
        vec![file("kvitto.pdf", "a")],
        today,
    )
    .await
    .unwrap();
    record_voucher(&pool, other, anna, sale("2025-03-01", 100), today)
        .await
        .unwrap();
    let sha = list_vouchers(&pool, id, anna, start).await.unwrap()[0].attachments[0]
        .sha256
        .clone();

    let (attachment, data) = get_attachment(&pool, id, anna, start, 1, &sha)
        .await
        .unwrap();
    assert_eq!(attachment.file_name.as_str(), "kvitto.pdf");
    assert_eq!(attachment.content_type, ContentType::Pdf);
    assert_eq!(data, file("", "a").data);

    // The right hash on another company's voucher, or on another voucher.
    for (company_id, number) in [(other, 1), (id, 2)] {
        assert!(matches!(
            get_attachment(&pool, company_id, anna, start, number, &sha).await,
            Err(Error::Domain(DomainError::AttachmentNotFound))
        ));
    }
    assert!(matches!(
        get_attachment(&pool, id, Uuid::new_v4(), start, 1, &sha).await,
        Err(Error::NotFound)
    ));

    // The same file in the other company: stored once, read there by its own name.
    add_attachment(&pool, other, anna, start, 1, file("kopia.pdf", "a"), today)
        .await
        .unwrap();
    let (theirs, _) = get_attachment(&pool, other, anna, start, 1, &sha)
        .await
        .unwrap();
    assert_eq!(theirs.file_name.as_str(), "kopia.pdf");
    assert_eq!(
        table(&pool, "SELECT sha256 FROM attachment_files")
            .await
            .len(),
        1
    );
}

fn post(lines: &[StatementLine], label: &str) -> (i64, Option<i64>) {
    let line = lines.iter().find(|l| l.label == label).unwrap();
    (line.amount, line.previous)
}

#[tokio::test]
async fn the_statements_compare_with_the_year_before() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let today = d(TODAY);
    for cmd in [sale("2025-03-01", 1_000), sale("2026-02-01", 50)] {
        record_voucher(&pool, id, anna, cmd, today).await.unwrap();
    }

    let first = financial_statements(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    assert_eq!(first.previous_fiscal_year_start, None);
    assert_eq!(post(&first.income, "Nettoomsättning"), (1_000, None));

    // 2025 is open, so 2026's balansräkning is short its result.
    let open = financial_statements(&pool, id, anna, d("2026-01-01"), today)
        .await
        .unwrap();
    assert_eq!(open.previous_fiscal_year_start, Some(d("2025-01-01")));
    assert_eq!(post(&open.income, "Nettoomsättning"), (50, Some(1_000)));
    assert_eq!(post(&open.balance, "Kassa och bank"), (1_050, Some(1_000)));
    assert_eq!(
        (open.difference, open.previous_difference),
        (1_000, Some(0))
    );

    close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    let closed = financial_statements(&pool, id, anna, d("2026-01-01"), today)
        .await
        .unwrap();
    assert_eq!(
        (closed.difference, closed.previous_difference),
        (0, Some(0))
    );
    // 2025's result is still on 2099, so 2026 shows it as balanserat.
    assert_eq!(post(&closed.balance, "Årets resultat"), (50, Some(1_000)));
    assert_eq!(
        post(&closed.balance, "Balanserat resultat"),
        (1_000, Some(0))
    );
}

#[tokio::test]
async fn the_statements_need_a_fiscal_year_start_and_membership() {
    let pool = db().await;
    let (anna, bo) = (Uuid::new_v4(), Uuid::new_v4());
    let id = company(&pool, anna).await;
    let today = d(TODAY);

    // Mid-year, before the first year, and after today.
    for start in ["2025-02-01", "2024-01-01", "2027-01-01", "9999-01-01"] {
        assert!(
            matches!(
                financial_statements(&pool, id, anna, d(start), today).await,
                Err(Error::Domain(DomainError::FiscalYearNotFound))
            ),
            "{start}"
        );
    }
    // Membership is checked before the year.
    assert!(matches!(
        financial_statements(&pool, id, bo, d("2025-02-01"), today).await,
        Err(Error::NotFound)
    ));
}
