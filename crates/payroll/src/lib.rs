//! Employees and payroll runs (lönekörningar) of a company, event-sourced
//! into SQLite. A run is booked as a ledger voucher in the same
//! transaction as its event.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, read
//! the corrected vouchers, load the company's payroll, decide, append,
//! project.

pub mod domain;
mod projections;
mod queries;

use domain::{
    AddEmployee, BookedVoucher, DomainError, EmployeeName, Payroll, PayrollEvent, PayrollRunDraft,
    PayrollRunLine, PersonalIdentityNumber, SalaryAccount, UpdateEmployee,
};
use doris_company::domain::Company;
use doris_eventstore::{Metadata, NewEvent};
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::HashSet;
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::{
    PayrollRunLineView, PayrollRunView, get_payroll_run, list_employees, list_payroll_runs,
};

const PAYROLL_STREAM: &str = "payroll-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// The ledger refused the voucher (closed year, inactive account, …).
    #[error(transparent)]
    Ledger(#[from] doris_ledger::Error),
    #[error(transparent)]
    Store(#[from] doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<doris_company::Error> for Error {
    fn from(err: doris_company::Error) -> Self {
        match err {
            doris_company::Error::Store(err) => Error::Store(err),
            _ => Error::NotFound,
        }
    }
}

/// Unvalidated input for a new employee, as it arrives from the API.
#[derive(Debug, Clone)]
pub struct NewEmployee<'a> {
    pub name: &'a str,
    pub personal_identity_number: &'a str,
    pub monthly_salary: i64,
    pub salary_account: u32,
}

pub async fn add_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    new: NewEmployee<'_>,
) -> Result<Uuid> {
    let cmd = AddEmployee {
        employee_id: Uuid::new_v4(),
        name: EmployeeName::parse(new.name)?,
        personal_identity_number: PersonalIdentityNumber::parse(new.personal_identity_number)?,
        monthly_salary: new.monthly_salary,
        salary_account: SalaryAccount::parse(new.salary_account)?,
    };
    let employee_id = cmd.employee_id;
    change(pool, company_id, actor, |payroll| {
        domain::add_employee(payroll, cmd)
    })
    .await?;
    Ok(employee_id)
}

pub async fn update_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
    name: &str,
    monthly_salary: i64,
    salary_account: u32,
) -> Result<()> {
    let cmd = UpdateEmployee {
        employee_id,
        name: EmployeeName::parse(name)?,
        monthly_salary,
        salary_account: SalaryAccount::parse(salary_account)?,
    };
    change(pool, company_id, actor, |payroll| {
        domain::update_employee(payroll, cmd)
    })
    .await
}

pub async fn deactivate_employee(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        domain::deactivate_employee(payroll, employee_id)
    })
    .await
}

/// What a draft would lock if finalized now.
#[derive(Debug, Clone, PartialEq)]
pub struct Preview {
    pub text: String,
    pub lines: Vec<PayrollRunLine>,
}

/// Computes a draft's lines without writing anything.
pub async fn preview_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    draft: PayrollRunDraft,
) -> Result<Preview> {
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    let draft = domain::validate_draft(&payroll, draft)?;
    let lines = domain::compute_lines(&payroll, None, &draft)?;
    Ok(Preview {
        text: draft.text,
        lines,
    })
}

pub async fn create_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    draft: PayrollRunDraft,
) -> Result<Uuid> {
    let payroll_run_id = Uuid::new_v4();
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::create_payroll_run(
            payroll,
            payroll_run_id,
            draft,
        )?])
    })
    .await?;
    Ok(payroll_run_id)
}

pub async fn update_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    draft: PayrollRunDraft,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::update_payroll_run(
            payroll,
            payroll_run_id,
            draft,
        )?])
    })
    .await
}

pub async fn finalize_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::finalize_payroll_run(payroll, payroll_run_id)?])
    })
    .await
}

pub async fn reopen_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![domain::reopen_payroll_run(payroll, payroll_run_id)?])
    })
    .await
}

/// One write of payroll events, decided on the company's current payroll.
async fn change(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Payroll) -> Result<Vec<PayrollEvent>, DomainError>,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let events = decide(&payroll)?;
    append(&mut tx, company_id, version, &events, actor).await?;
    tx.commit().await?;
    Ok(())
}

fn payroll_stream(company_id: Uuid) -> String {
    format!("{PAYROLL_STREAM}{company_id}")
}

/// The company (checking membership), its payroll and the stream version.
async fn load(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
) -> Result<(Company, Payroll, i64)> {
    let company = doris_company::get_company_in(conn, company_id, actor).await?;
    let reversed = reversed_vouchers(conn, company_id).await?;
    let recorded = doris_eventstore::load(conn, &payroll_stream(company_id)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<PayrollEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((company, Payroll::from_events(&events, reversed), version))
}

/// The company's vouchers that a rättelse points at, from the ledger's
/// projection. A rättelse is always in its original's fiscal year.
async fn reversed_vouchers(
    conn: &mut SqliteConnection,
    company_id: Uuid,
) -> Result<HashSet<BookedVoucher>> {
    let rows: Vec<(String, u32)> = sqlx::query_as(
        "SELECT fiscal_year_start, corrects FROM vouchers
         WHERE company_id = ? AND corrects IS NOT NULL",
    )
    .bind(company_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(start, number)| BookedVoucher {
            fiscal_year_start: start.parse().expect("the ledger stores YYYY-MM-DD"),
            number,
        })
        .collect())
}

/// Appends events and updates projections within the caller's transaction.
async fn append(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    expected_version: i64,
    events: &[PayrollEvent],
    actor: Uuid,
) -> Result<()> {
    if events.is_empty() {
        return Ok(());
    }
    let new_events = events
        .iter()
        .map(|e| NewEvent::from_tagged(e, SCHEMA_VERSION))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata = Metadata {
        actor: Some(actor.to_string()),
    };
    let stream = payroll_stream(company_id);
    let recorded =
        doris_eventstore::append(conn, &stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
