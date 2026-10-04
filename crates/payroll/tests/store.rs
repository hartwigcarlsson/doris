use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_payroll::domain::DomainError;
use doris_payroll::{
    Error, NewEmployee, add_employee, book_payroll_run, create_payroll_run, deactivate_employee,
    finalize_payroll_run, get_payroll_run, list_employees, list_payroll_runs, preview_payroll_run,
    rebuild_projections, reopen_payroll_run, unbook_payroll_run, update_employee,
    update_payroll_run,
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
        (35_000 * KR, 8_000 * KR, None)
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
