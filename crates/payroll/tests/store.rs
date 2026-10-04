use doris_company::NewCompany;
use doris_company::domain::{AccountingMethod, LegalForm};
use doris_payroll::domain::DomainError;
use doris_payroll::{
    Error, NewEmployee, add_employee, create_payroll_run, deactivate_employee,
    finalize_payroll_run, get_payroll_run, list_employees, list_payroll_runs, preview_payroll_run,
    rebuild_projections, reopen_payroll_run, update_employee, update_payroll_run,
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
                tax,
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
