use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_payroll::domain::DomainError;
use doris_payroll::{
    Error, NewEmployee, add_employee, agi_contact, agi_file, agi_month, agi_months,
    book_payroll_run, create_payroll_run, deactivate_employee, finalize_payroll_run,
    get_payroll_run, list_employees, list_payroll_runs, preview_payroll_run, rebuild_projections,
    reopen_payroll_run, set_agi_contact, set_employee_tax, store_tax_table, submit_agi_month,
    tax_table, unbook_payroll_run, update_employee, update_payroll_run,
};
use sqlx::SqlitePool;
use uuid::Uuid;

const KR: i64 = 100;

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

fn asa() -> NewEmployee<'static> {
    NewEmployee {
        name: "Åsa Öberg",
        personal_identity_number: "19800101-1231",
        monthly_salary: 35_000 * KR,
        salary_account: 7210,
        tax: None,
    }
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
async fn employees_are_added_updated_deactivated_and_listed_by_name() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;

    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let bo = NewEmployee {
        name: "Bo Ek",
        personal_identity_number: "198507099870",
        monthly_salary: 30_000 * KR,
        salary_account: 7010,
        tax: None,
    };
    let bo_id = add_employee(&pool, id, anna, bo).await.unwrap();
    update_employee(&pool, id, anna, asa_id, "Åsa Öberg Lind", 36_000 * KR, 7220)
        .await
        .unwrap();
    deactivate_employee(&pool, id, anna, bo_id).await.unwrap();

    let employees = list_employees(&pool, id, anna).await.unwrap();
    let rows: Vec<_> = employees
        .iter()
        .map(|e| {
            (
                e.name.as_str(),
                e.personal_identity_number.formatted(),
                e.monthly_salary,
                e.salary_account.get(),
                e.active,
            )
        })
        .collect();
    assert_eq!(
        rows,
        vec![
            (
                "Bo Ek",
                "19850709-9870".to_owned(),
                30_000 * KR,
                7010,
                false
            ),
            (
                "Åsa Öberg Lind",
                "19800101-1231".to_owned(),
                36_000 * KR,
                7220,
                true
            ),
        ]
    );
    assert_eq!(
        events_of(&pool, "payroll-").await,
        [
            "EmployeeAdded",
            "EmployeeAdded",
            "EmployeeUpdated",
            "EmployeeDeactivated"
        ]
    );
}

#[tokio::test]
async fn invalid_or_duplicate_employees_are_refused_and_write_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    add_employee(&pool, id, anna, asa()).await.unwrap();

    let refused = |e: Error| match e {
        Error::Domain(e) => e,
        other => panic!("{other:?}"),
    };
    let again = add_employee(&pool, id, anna, asa()).await.unwrap_err();
    assert_eq!(refused(again), DomainError::DuplicateEmployee);
    let bad_pin = NewEmployee {
        personal_identity_number: "19800101-1232",
        ..asa()
    };
    assert_eq!(
        refused(add_employee(&pool, id, anna, bad_pin).await.unwrap_err()),
        DomainError::InvalidPersonalIdentityNumber
    );
    let bad_account = NewEmployee {
        personal_identity_number: "19850709-9870",
        salary_account: 7510,
        ..asa()
    };
    assert_eq!(
        refused(
            add_employee(&pool, id, anna, bad_account)
                .await
                .unwrap_err()
        ),
        DomainError::InvalidSalaryAccount
    );
    assert_eq!(
        refused(
            update_employee(&pool, id, anna, Uuid::new_v4(), "X", 1, 7210)
                .await
                .unwrap_err()
        ),
        DomainError::EmployeeNotFound
    );

    assert_eq!(events_of(&pool, "payroll-").await, ["EmployeeAdded"]);
}

#[tokio::test]
async fn the_database_refuses_a_duplicate_personnummer() {
    let pool = db().await;
    let insert = "INSERT INTO employees (company_id, employee_id, name, personal_identity_number,
                  monthly_salary, salary_account, active) VALUES ('c', ?, 'X', '198001011231', 1, 7210, 1)";
    sqlx::query(insert).bind("a").execute(&pool).await.unwrap();
    assert!(sqlx::query(insert).bind("b").execute(&pool).await.is_err());
}

#[tokio::test]
async fn non_members_get_not_found() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let eve = Uuid::new_v4();

    assert!(matches!(
        add_employee(&pool, id, eve, asa()).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_employees(&pool, id, eve).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_employees(&pool, Uuid::new_v4(), anna).await,
        Err(Error::NotFound)
    ));
}

#[tokio::test]
async fn the_employee_projection_rebuilds_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    update_employee(&pool, id, anna, asa_id, "Åsa Lind", 36_000 * KR, 7220)
        .await
        .unwrap();
    deactivate_employee(&pool, id, anna, asa_id).await.unwrap();
    let sql = "SELECT company_id || employee_id || name || personal_identity_number
               || monthly_salary || salary_account || active FROM employees ORDER BY 1";
    let before = table(&pool, sql).await;

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(table(&pool, sql).await, before);
    assert_eq!(before.len(), 1);
}

use doris_payroll::domain::{
    DraftLine, FULL_RATE, PayrollRunDraft, PayrollRunLine, PayrollRunStatus, SalaryAccount,
};
use doris_payroll::tax::TaxBasis;
use jiff::civil::Date;

fn d(s: &str) -> Date {
    s.parse().unwrap()
}

fn draft(pay_date: &str, lines: &[(Uuid, i64, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date: d(pay_date),
        text: String::new(),
        lines: lines
            .iter()
            .map(|&(employee_id, gross, tax)| DraftLine {
                employee_id,
                gross,
                tax: Some(tax),
            })
            .collect(),
    }
}

#[tokio::test]
async fn a_preview_computes_the_lines_and_writes_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();

    let preview = preview_payroll_run(
        &pool,
        id,
        anna,
        draft("2026-10-25", &[(asa_id, 35_000 * KR, 8_000 * KR)]),
    )
    .await
    .unwrap();

    assert_eq!(preview.text, "Lön oktober 2026");
    assert_eq!(
        preview.lines,
        [PayrollRunLine {
            employee_id: asa_id,
            salary_account: SalaryAccount::DEFAULT,
            gross: 35_000 * KR,
            tax: 8_000 * KR,
            fee_rate: FULL_RATE,
            fee: 1_099_700,
            net: 27_000 * KR,
            tax_basis: TaxBasis::Manual,
        }]
    );
    assert_eq!(events_of(&pool, "payroll-").await, ["EmployeeAdded"]);
}

#[tokio::test]
async fn a_run_is_created_changed_finalized_and_reopened() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();

    let run = create_payroll_run(
        &pool,
        id,
        anna,
        draft("2026-10-25", &[(asa_id, 35_000 * KR, 8_000 * KR)]),
    )
    .await
    .unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Open);
    assert_eq!(view.text, "Lön oktober 2026");
    assert_eq!(view.lines[0].employee_name, "Åsa Öberg");
    assert_eq!(
        (view.lines[0].gross, view.lines[0].tax, view.lines[0].locked),
        (35_000 * KR, Some(8_000 * KR), None)
    );
    assert!(view.voucher_lines().is_empty());

    update_payroll_run(
        &pool,
        id,
        anna,
        run,
        draft("2026-10-24", &[(asa_id, 36_000 * KR, 8_300 * KR)]),
    )
    .await
    .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Finalized);
    assert_eq!(view.pay_date, d("2026-10-24"));
    let locked = view.lines[0].locked.unwrap();
    assert_eq!(
        (locked.gross, locked.fee, locked.net),
        (36_000 * KR, 1_131_120, 27_700 * KR)
    );
    assert_eq!(view.voucher_lines().len(), 5);

    // Finalized: no changes until it is opened again.
    let refused = update_payroll_run(&pool, id, anna, run, draft("2026-10-24", &[(asa_id, 1, 0)]))
        .await
        .unwrap_err();
    assert!(matches!(
        refused,
        Error::Domain(DomainError::PayrollRunNotOpen)
    ));

    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Open);
    assert_eq!(view.lines[0].locked, None);
    assert_eq!(view.lines[0].gross, 36_000 * KR);

    assert_eq!(
        events_of(&pool, "payroll-").await,
        [
            "EmployeeAdded",
            "PayrollRunCreated",
            "PayrollRunUpdated",
            "PayrollRunFinalized",
            "PayrollRunReopened"
        ]
    );
}

#[tokio::test]
async fn runs_are_listed_newest_pay_date_first() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    for pay_date in ["2026-09-25", "2026-11-25", "2026-10-25"] {
        create_payroll_run(&pool, id, anna, draft(pay_date, &[(asa_id, 100, 0)]))
            .await
            .unwrap();
    }

    let runs = list_payroll_runs(&pool, id, anna).await.unwrap();

    let dates: Vec<_> = runs.iter().map(|r| r.pay_date.to_string()).collect();
    assert_eq!(dates, ["2026-11-25", "2026-10-25", "2026-09-25"]);
}

#[tokio::test]
async fn a_missing_run_or_a_stranger_finds_nothing() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let run = create_payroll_run(&pool, id, anna, draft("2026-10-25", &[(asa_id, 100, 0)]))
        .await
        .unwrap();
    let eve = Uuid::new_v4();

    assert!(matches!(
        get_payroll_run(&pool, id, anna, Uuid::new_v4()).await,
        Err(Error::Domain(DomainError::PayrollRunNotFound))
    ));
    assert!(matches!(
        get_payroll_run(&pool, id, eve, run).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        list_payroll_runs(&pool, id, eve).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        finalize_payroll_run(&pool, id, eve, run).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        preview_payroll_run(&pool, id, eve, draft("2026-10-25", &[(asa_id, 100, 0)])).await,
        Err(Error::NotFound)
    ));
}

use doris_ledger::domain::DomainError as LedgerError;

/// Åsa and a finalized run paying her 35 000 kr on `pay_date`.
async fn finalized_run(pool: &SqlitePool, id: Uuid, anna: Uuid, pay_date: &str) -> Uuid {
    let asa_id = add_employee(pool, id, anna, asa()).await.unwrap();
    let run = create_payroll_run(
        pool,
        id,
        anna,
        draft(pay_date, &[(asa_id, 35_000 * KR, 8_000 * KR)]),
    )
    .await
    .unwrap();
    finalize_payroll_run(pool, id, anna, run).await.unwrap();
    run
}

fn ledger_refusal(err: Error) -> LedgerError {
    match err {
        Error::Ledger(doris_ledger::Error::Domain(e)) => e,
        other => panic!("{other:?}"),
    }
}

#[tokio::test]
async fn a_run_books_on_its_pay_date_and_not_before() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;

    let early = book_payroll_run(&pool, id, anna, run, d("2025-10-24"))
        .await
        .unwrap_err();
    assert!(matches!(
        early,
        Error::Domain(DomainError::PayrollRunNotDue)
    ));
    assert!(events_of(&pool, "ledger-").await.is_empty());

    let voucher = book_payroll_run(&pool, id, anna, run, d("2025-10-25"))
        .await
        .unwrap();

    assert_eq!(
        (voucher.fiscal_year_start, voucher.number),
        (d("2025-01-01"), 1)
    );
    assert_eq!(events_of(&pool, "ledger-").await, ["VoucherRecorded"]);
    assert_eq!(
        events_of(&pool, "payroll-").await.last().unwrap(),
        "PayrollRunBooked"
    );
    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    let lines: Vec<_> = vouchers[0]
        .lines
        .iter()
        .map(|l| (l.account.get(), l.debit, l.credit))
        .collect();
    assert_eq!(
        lines,
        [
            (7210, 35_000 * KR, 0),
            (2710, 0, 8_000 * KR),
            (1930, 0, 27_000 * KR),
            (7510, 1_099_700, 0),
            (2731, 0, 1_099_700),
        ]
    );
    assert_eq!(
        (vouchers[0].date, vouchers[0].text.as_str()),
        (d("2025-10-25"), "Lön oktober 2025")
    );
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Booked(voucher));
    assert!(matches!(
        reopen_payroll_run(&pool, id, anna, run).await,
        Err(Error::Domain(DomainError::PayrollRunBooked))
    ));
}

#[tokio::test]
async fn a_refused_booking_writes_nothing_and_the_run_stays_finalized() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-25");
    let payroll_events = events_of(&pool, "payroll-").await;

    doris_ledger::set_account_active(&pool, id, anna, 7210, false)
        .await
        .unwrap();
    let refused = book_payroll_run(&pool, id, anna, run, today)
        .await
        .unwrap_err();
    assert_eq!(ledger_refusal(refused), LedgerError::AccountInactive);
    assert_eq!(events_of(&pool, "payroll-").await, payroll_events);
    assert!(events_of(&pool, "ledger-").await.is_empty());
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();
    assert_eq!(view.status, PayrollRunStatus::Finalized);

    doris_ledger::set_account_active(&pool, id, anna, 7210, true)
        .await
        .unwrap();
    assert_eq!(
        book_payroll_run(&pool, id, anna, run, today)
            .await
            .unwrap()
            .number,
        1
    );
}

#[tokio::test]
async fn a_closed_year_refuses_the_booking() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-12-25").await;
    let today = d("2026-01-10");
    doris_ledger::close_fiscal_year(&pool, id, anna, d("2025-01-01"), today)
        .await
        .unwrap();
    let ledger_events = events_of(&pool, "ledger-").await;

    let refused = book_payroll_run(&pool, id, anna, run, today)
        .await
        .unwrap_err();

    assert_eq!(ledger_refusal(refused), LedgerError::FiscalYearClosed);
    assert_eq!(events_of(&pool, "ledger-").await, ledger_events);
}

#[tokio::test]
async fn backa_bokforing_reverses_the_voucher_and_the_run_can_change_and_book_again() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    let first = book_payroll_run(&pool, id, anna, run, today).await.unwrap();

    let correction = unbook_payroll_run(&pool, id, anna, run, today)
        .await
        .unwrap();

    assert_eq!(correction.number, 2);
    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    assert_eq!(vouchers[1].corrects, Some(first.number));
    assert_eq!(vouchers[1].date, today);
    assert_eq!(
        get_payroll_run(&pool, id, anna, run).await.unwrap().status,
        PayrollRunStatus::Finalized
    );
    assert!(matches!(
        unbook_payroll_run(&pool, id, anna, run, today).await,
        Err(Error::Domain(DomainError::PayrollRunNotBooked))
    ));

    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
    let asa_id = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].employee_id;
    update_payroll_run(
        &pool,
        id,
        anna,
        run,
        draft("2025-10-25", &[(asa_id, 36_000 * KR, 8_300 * KR)]),
    )
    .await
    .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let again = book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    assert_eq!(again.number, 3);
    assert_eq!(
        get_payroll_run(&pool, id, anna, run).await.unwrap().status,
        PayrollRunStatus::Booked(again)
    );
}

#[tokio::test]
async fn a_rattelse_from_the_grundbok_also_unbooks_the_run() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    let voucher = book_payroll_run(&pool, id, anna, run, today).await.unwrap();

    doris_ledger::correct_voucher(
        &pool,
        id,
        anna,
        voucher.fiscal_year_start,
        voucher.number,
        today,
        today,
    )
    .await
    .unwrap();

    assert_eq!(
        get_payroll_run(&pool, id, anna, run).await.unwrap().status,
        PayrollRunStatus::Finalized
    );
    reopen_payroll_run(&pool, id, anna, run).await.unwrap();
}

#[tokio::test]
async fn unbooking_after_the_year_ended_dates_the_rattelse_on_its_last_day() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-12-25").await;
    book_payroll_run(&pool, id, anna, run, d("2025-12-27"))
        .await
        .unwrap();

    unbook_payroll_run(&pool, id, anna, run, d("2026-01-10"))
        .await
        .unwrap();

    let vouchers = doris_ledger::list_vouchers(&pool, id, anna, d("2025-01-01"))
        .await
        .unwrap();
    assert_eq!(vouchers[1].date, d("2025-12-31"));
}

#[tokio::test]
async fn all_projections_rebuild_with_bookings_in_place() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = finalized_run(&pool, id, anna, "2025-10-25").await;
    let today = d("2025-10-27");
    book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    unbook_payroll_run(&pool, id, anna, run, today)
        .await
        .unwrap();
    book_payroll_run(&pool, id, anna, run, today).await.unwrap();
    let runs = "SELECT company_id || payroll_run_id || pay_date || text || finalized || updated_by
                FROM payroll_runs ORDER BY 1";
    let lines = "SELECT payroll_run_id || employee_id || gross || tax
                 || COALESCE(salary_account, '-') || COALESCE(fee, '-') || COALESCE(net, '-')
                 FROM payroll_run_lines ORDER BY 1";
    let bookings = "SELECT payroll_run_id || fiscal_year_start || voucher_number
                    FROM payroll_run_bookings ORDER BY rowid";
    let before = (
        table(&pool, runs).await,
        table(&pool, lines).await,
        table(&pool, bookings).await,
    );
    let view = get_payroll_run(&pool, id, anna, run).await.unwrap();

    // The ledger's rebuild empties vouchers: no payroll key may refer to it.
    doris_ledger::rebuild_projections(&pool).await.unwrap();
    rebuild_projections(&pool).await.unwrap();

    let after = (
        table(&pool, runs).await,
        table(&pool, lines).await,
        table(&pool, bookings).await,
    );
    assert_eq!(after, before);
    assert_eq!(before.2.len(), 2);
    assert_eq!(get_payroll_run(&pool, id, anna, run).await.unwrap(), view);
}

use doris_payroll::tax::{RowKind, TaxSetting, TaxTable, TaxTableRow};

/// Every table 29–42 with one amount band (1–80 000 kr, `kronor` in each
/// column) and one open 40 % band.
fn flat_table(year: i16, kronor: i64) -> TaxTable {
    let rows = (29..=42u8)
        .flat_map(|table| {
            [
                TaxTableRow {
                    table,
                    kind: RowKind::Amount,
                    from: 1,
                    to: Some(80_000),
                    columns: [kronor; 6],
                },
                TaxTableRow {
                    table,
                    kind: RowKind::Percent,
                    from: 80_001,
                    to: None,
                    columns: [40; 6],
                },
            ]
        })
        .collect();
    TaxTable::validate(year, rows).unwrap()
}

fn computed(pay_date: &str, lines: &[(Uuid, i64)]) -> PayrollRunDraft {
    PayrollRunDraft {
        pay_date: d(pay_date),
        text: String::new(),
        lines: lines
            .iter()
            .map(|&(employee_id, gross)| DraftLine {
                employee_id,
                gross,
                tax: None,
            })
            .collect(),
    }
}

/// Åsa on tabell 33, kolumn 1.
async fn asa_on_table(pool: &SqlitePool, id: Uuid, anna: Uuid) -> Uuid {
    let new = NewEmployee {
        tax: Some(TaxSetting::table(33, 1).unwrap()),
        ..asa()
    };
    add_employee(pool, id, anna, new).await.unwrap()
}

#[tokio::test]
async fn a_stored_year_reads_back_and_storing_it_again_replaces_it() {
    let pool = db().await;
    assert_eq!(tax_table(&pool, 2026).await.unwrap(), None);

    store_tax_table(&pool, &flat_table(2026, 7_000))
        .await
        .unwrap();
    store_tax_table(&pool, &flat_table(2026, 7_100))
        .await
        .unwrap();

    assert_eq!(
        tax_table(&pool, 2026).await.unwrap(),
        Some(flat_table(2026, 7_100))
    );
    assert_eq!(
        table(&pool, "SELECT COUNT(*) || '' FROM tax_tables").await,
        ["28"]
    );
    // Another year is not this one.
    assert_eq!(tax_table(&pool, 2027).await.unwrap(), None);
}

#[tokio::test]
async fn employees_carry_their_tax_setting() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;

    assert_eq!(
        list_employees(&pool, id, anna).await.unwrap()[0].tax,
        Some(TaxSetting::Table {
            table: 33,
            column: 1
        })
    );
    set_employee_tax(&pool, id, anna, asa_id, TaxSetting::percent(30).unwrap())
        .await
        .unwrap();
    assert_eq!(
        list_employees(&pool, id, anna).await.unwrap()[0].tax,
        Some(TaxSetting::Percent { percent: 30 })
    );
    assert_eq!(
        events_of(&pool, "payroll-").await,
        ["EmployeeAdded", "EmployeeTaxChanged", "EmployeeTaxChanged"]
    );
}

#[tokio::test]
async fn a_blank_tax_needs_the_years_table_and_nothing_is_written_without_it() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    let run = create_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-01-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    assert_eq!(
        get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].tax,
        None
    );
    // Only last year's table is stored: January needs the new year's.
    store_tax_table(&pool, &flat_table(2025, 6_000))
        .await
        .unwrap();
    let before = events_of(&pool, "payroll-").await;

    let missing = finalize_payroll_run(&pool, id, anna, run)
        .await
        .unwrap_err();
    assert!(matches!(
        missing,
        Error::Domain(DomainError::TaxTableMissing(2026))
    ));
    assert!(matches!(
        preview_payroll_run(
            &pool,
            id,
            anna,
            computed("2026-01-25", &[(asa_id, 35_000 * KR)])
        )
        .await,
        Err(Error::Domain(DomainError::TaxTableMissing(2026)))
    ));
    assert_eq!(events_of(&pool, "payroll-").await, before);

    store_tax_table(&pool, &flat_table(2026, 7_000))
        .await
        .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let line = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0]
        .locked
        .unwrap();
    assert_eq!(
        (line.tax, line.tax_basis),
        (
            7_000 * KR,
            TaxBasis::Table {
                year: 2026,
                table: 33,
                column: 1
            }
        )
    );
}

#[tokio::test]
async fn a_changed_setting_moves_an_open_run_but_not_a_finalized_one() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    store_tax_table(&pool, &flat_table(2026, 7_000))
        .await
        .unwrap();
    let locked = create_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-01-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    finalize_payroll_run(&pool, id, anna, locked).await.unwrap();

    set_employee_tax(&pool, id, anna, asa_id, TaxSetting::percent(30).unwrap())
        .await
        .unwrap();

    let preview = preview_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-02-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    assert_eq!(
        (preview.lines[0].tax, preview.lines[0].tax_basis),
        (10_500 * KR, TaxBasis::Percent { percent: 30 })
    );
    let kept = get_payroll_run(&pool, id, anna, locked)
        .await
        .unwrap()
        .lines[0]
        .locked
        .unwrap();
    assert_eq!(
        (kept.tax, kept.tax_basis),
        (
            7_000 * KR,
            TaxBasis::Table {
                year: 2026,
                table: 33,
                column: 1
            }
        )
    );
}

#[tokio::test]
async fn a_run_finalized_before_tax_bases_reads_as_manual() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = add_employee(&pool, id, anna, asa()).await.unwrap();
    let run = Uuid::new_v4();
    // Events as step 9 wrote them: a number for tax, no tax_basis.
    let old = [
        format!(
            r#"{{"type":"PayrollRunCreated","payroll_run_id":"{run}","draft":{{"pay_date":"2025-10-25","text":"Lön oktober 2025","lines":[{{"employee_id":"{asa_id}","gross":3500000,"tax":800000}}]}}}}"#
        ),
        format!(
            r#"{{"type":"PayrollRunFinalized","payroll_run_id":"{run}","lines":[{{"employee_id":"{asa_id}","salary_account":7210,"gross":3500000,"tax":800000,"fee_rate":3142,"fee":1099700,"net":2700000}}]}}"#
        ),
    ];
    for (version, payload) in (2..).zip(old) {
        let event_type = if version == 2 {
            "PayrollRunCreated"
        } else {
            "PayrollRunFinalized"
        };
        sqlx::query(
            "INSERT INTO events (stream_id, stream_version, event_type, schema_version, payload, metadata)
             VALUES (?, ?, ?, 1, ?, '{\"actor\":null}')",
        )
        .bind(format!("payroll-{id}"))
        .bind(version)
        .bind(event_type)
        .bind(payload)
        .execute(&pool)
        .await
        .unwrap();
    }
    rebuild_projections(&pool).await.unwrap();

    let line = get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0]
        .locked
        .unwrap();
    assert_eq!((line.tax, line.tax_basis), (800_000, TaxBasis::Manual));
    // And it still books.
    let voucher = book_payroll_run(&pool, id, anna, run, d("2025-10-25"))
        .await
        .unwrap();
    assert_eq!(voucher.number, 1);
}

#[tokio::test]
async fn tax_settings_and_bases_rebuild_from_the_events() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    store_tax_table(&pool, &flat_table(2026, 7_000))
        .await
        .unwrap();
    let run = create_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-01-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    finalize_payroll_run(&pool, id, anna, run).await.unwrap();
    let open = create_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-02-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    let employees = "SELECT employee_id || ':' || COALESCE(tax_table, '-') || ':'
                     || COALESCE(tax_column, '-') || ':' || COALESCE(tax_percent, '-') FROM employees";
    let lines =
        "SELECT payroll_run_id || ':' || COALESCE(tax, '-') || ':' || COALESCE(tax_basis, '-')
                 FROM payroll_run_lines ORDER BY 1";
    let before = (table(&pool, employees).await, table(&pool, lines).await);

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(
        (table(&pool, employees).await, table(&pool, lines).await),
        before
    );
    assert_eq!(
        get_payroll_run(&pool, id, anna, open).await.unwrap().lines[0].tax,
        None
    );
}

#[tokio::test]
async fn reopening_forgets_a_computed_tax_but_keeps_a_typed_one() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let asa_id = asa_on_table(&pool, id, anna).await;
    store_tax_table(&pool, &flat_table(2026, 7_000))
        .await
        .unwrap();
    let computed_run = create_payroll_run(
        &pool,
        id,
        anna,
        computed("2026-01-25", &[(asa_id, 35_000 * KR)]),
    )
    .await
    .unwrap();
    let typed_run = create_payroll_run(
        &pool,
        id,
        anna,
        draft("2026-02-25", &[(asa_id, 35_000 * KR, 8_000 * KR)]),
    )
    .await
    .unwrap();
    for run in [computed_run, typed_run] {
        finalize_payroll_run(&pool, id, anna, run).await.unwrap();
        reopen_payroll_run(&pool, id, anna, run).await.unwrap();
    }
    let tax_of = |run| {
        let pool = pool.clone();
        async move { get_payroll_run(&pool, id, anna, run).await.unwrap().lines[0].tax }
    };
    assert_eq!(tax_of(computed_run).await, None);
    assert_eq!(tax_of(typed_run).await, Some(8_000 * KR));
    let rows = "SELECT COALESCE(tax, '-') || ':' || COALESCE(tax_basis, '-') FROM payroll_run_lines ORDER BY 1";
    let before = table(&pool, rows).await;
    assert!(before.iter().any(|r| r == "-:-"));

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(table(&pool, rows).await, before);
}

use doris_payroll::agi::{AgiStatus, Period};

/// Åsa paid 35 000 kr with 8 000 kr tax on 2025-10-25, booked.
async fn booked_october(pool: &SqlitePool, id: Uuid, anna: Uuid) -> Uuid {
    let run = finalized_run(pool, id, anna, "2025-10-25").await;
    book_payroll_run(pool, id, anna, run, d("2025-10-25"))
        .await
        .unwrap();
    run
}

fn oct25() -> Period {
    Period::parse("202510").unwrap()
}

#[tokio::test]
async fn a_month_is_declared_and_changes_are_noticed() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let run = booked_october(&pool, id, anna).await;

    let months = agi_months(&pool, id, anna).await.unwrap();
    assert_eq!(months.len(), 1);
    assert_eq!(
        (
            months[0].period,
            months[0].gross,
            months[0].tax_sum,
            months[0].fee_sum,
            months[0].status
        ),
        (oct25(), 35_000, 8_000, 10_997, AgiStatus::NotSubmitted)
    );
    assert!(matches!(
        agi_file(
            &pool,
            id,
            anna,
            oct25(),
            jiff::civil::date(2025, 11, 5).at(9, 0, 0, 0)
        )
        .await,
        Err(Error::Domain(DomainError::AgiContactMissing))
    ));

    set_agi_contact(
        &pool,
        id,
        anna,
        "Anna Andersson",
        "070-123 45 67",
        "anna@example.se",
    )
    .await
    .unwrap();
    let (name, xml) = agi_file(
        &pool,
        id,
        anna,
        oct25(),
        jiff::civil::date(2025, 11, 5).at(9, 0, 0, 0),
    )
    .await
    .unwrap();
    assert_eq!(name, "AGI_165560160680_202510.xml");
    assert!(xml.contains(
        r#"<agd:BetalningsmottagarId faltkod="215">198001011231</agd:BetalningsmottagarId>"#
    ));
    assert!(xml.contains(r#"<agd:SummaArbAvgSlf faltkod="487">10997</agd:SummaArbAvgSlf>"#));

    submit_agi_month(&pool, id, anna, oct25()).await.unwrap();
    let months = agi_months(&pool, id, anna).await.unwrap();
    assert_eq!(months[0].status, AgiStatus::Submitted);
    assert!(months[0].submitted_at.is_some());
    assert!(matches!(
        submit_agi_month(&pool, id, anna, oct25()).await,
        Err(Error::Domain(DomainError::AgiUnchanged))
    ));

    unbook_payroll_run(&pool, id, anna, run, d("2025-10-27"))
        .await
        .unwrap();
    let month = agi_month(&pool, id, anna, oct25()).await.unwrap();
    assert_eq!(month.status, AgiStatus::Changed);
    assert_eq!(month.removed.len(), 1);
    let (_, xml) = agi_file(
        &pool,
        id,
        anna,
        oct25(),
        jiff::civil::date(2025, 11, 5).at(9, 0, 0, 0),
    )
    .await
    .unwrap();
    assert!(xml.contains(r#"<agd:Borttag faltkod="205">1</agd:Borttag>"#));
    assert!(xml.contains("198001011231"));
}

#[tokio::test]
async fn the_contact_is_checked_and_kept() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    assert_eq!(agi_contact(&pool, id, anna).await.unwrap(), None);
    assert!(matches!(
        set_agi_contact(&pool, id, anna, "Anna", "", "anna@example.se").await,
        Err(Error::Domain(DomainError::InvalidAgiContact))
    ));
    set_agi_contact(&pool, id, anna, "Anna", "070", "anna@example.se")
        .await
        .unwrap();
    set_agi_contact(&pool, id, anna, "Anna", "070", "anna@example.se")
        .await
        .unwrap();
    assert_eq!(
        agi_contact(&pool, id, anna).await.unwrap().unwrap().phone,
        "070"
    );
    assert_eq!(events_of(&pool, "payroll-").await, ["AgiContactChanged"]);
}

#[tokio::test]
async fn agi_projections_rebuild_and_record_who_submitted() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    booked_october(&pool, id, anna).await;
    set_agi_contact(&pool, id, anna, "Anna", "070", "anna@example.se")
        .await
        .unwrap();
    submit_agi_month(&pool, id, anna, oct25()).await.unwrap();
    let contacts = "SELECT company_id || name || phone || email FROM agi_contacts";
    let submissions =
        "SELECT period || ':' || submitted_by || ':' || lines || ':' || fee_sum || ':' || tax_sum
                       FROM agi_submissions ORDER BY rowid";
    let before = (
        table(&pool, contacts).await,
        table(&pool, submissions).await,
    );
    assert!(before.1[0].contains(&anna.to_string()));

    rebuild_projections(&pool).await.unwrap();

    assert_eq!(
        (
            table(&pool, contacts).await,
            table(&pool, submissions).await
        ),
        before
    );
}

#[tokio::test]
async fn strangers_see_no_agi() {
    let pool = db().await;
    let anna = Uuid::new_v4();
    let id = company(&pool, anna).await;
    let eve = Uuid::new_v4();
    assert!(matches!(
        agi_months(&pool, id, eve).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        agi_contact(&pool, id, eve).await,
        Err(Error::NotFound)
    ));
    assert!(matches!(
        submit_agi_month(&pool, id, eve, oct25()).await,
        Err(Error::NotFound)
    ));
}
