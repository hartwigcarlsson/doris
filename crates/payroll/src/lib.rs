//! Employees and payroll runs (lönekörningar) of a company, event-sourced
//! into SQLite. A run is booked as a ledger voucher in the same
//! transaction as its event.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, read
//! the corrected vouchers, load the company's payroll, decide, append,
//! project.

pub mod agi;
pub mod domain;
mod projections;
mod queries;
pub mod tax;

use crate::tax::{RowKind, TaxSetting, TaxTable, TaxTableRow};
use agi::{AgiContact, AgiMonth, AgiStatus, Period};
use domain::{
    AddEmployee, BookedVoucher, DomainError, EmployeeName, Payroll, PayrollEvent, PayrollRunDraft,
    PayrollRunLine, PersonalIdentityNumber, SalaryAccount, UpdateEmployee,
};
use doris_company::domain::Company;
use doris_eventstore::{Metadata, NewEvent};
use jiff::civil::Date;
use sqlx::{SqliteConnection, SqlitePool};
use std::collections::{HashMap, HashSet};
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
    pub tax: Option<TaxSetting>,
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
        tax: new.tax,
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
    let table = load_tax_table(&mut conn, draft.pay_date.year()).await?;
    let lines = domain::compute_lines(&payroll, None, &draft, table.as_ref())?;
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
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let table = match payroll.run(payroll_run_id) {
        Some(run) => load_tax_table(&mut tx, run.draft.pay_date.year()).await?,
        None => None,
    };
    let event = domain::finalize_payroll_run(&payroll, payroll_run_id, table.as_ref())?;
    append(&mut tx, company_id, version, &[event], actor).await?;
    tx.commit().await?;
    Ok(())
}

pub async fn set_employee_tax(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    employee_id: Uuid,
    tax: TaxSetting,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        domain::set_employee_tax(payroll, employee_id, tax)
    })
    .await
}

/// The stored monthly tables for `year`, if any.
pub async fn tax_table(pool: &SqlitePool, year: i16) -> Result<Option<TaxTable>> {
    let mut conn = pool.acquire().await?;
    load_tax_table(&mut conn, year).await
}

/// Stores a year's tables, replacing any earlier copy, in one transaction.
pub async fn store_tax_table(pool: &SqlitePool, table: &TaxTable) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    sqlx::query("DELETE FROM tax_tables WHERE year = ?")
        .bind(table.year())
        .execute(&mut *tx)
        .await?;
    for row in table.rows() {
        let kind = match row.kind {
            RowKind::Amount => "amount",
            RowKind::Percent => "percent",
        };
        let [c1, c2, c3, c4, c5, c6] = row.columns;
        sqlx::query(
            "INSERT INTO tax_tables (year, table_no, kind, income_from, income_to,
                 col1, col2, col3, col4, col5, col6)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
        )
        .bind(table.year())
        .bind(row.table)
        .bind(kind)
        .bind(row.from)
        .bind(row.to)
        .bind(c1)
        .bind(c2)
        .bind(c3)
        .bind(c4)
        .bind(c5)
        .bind(c6)
        .execute(&mut *tx)
        .await?;
    }
    tx.commit().await?;
    Ok(())
}

/// `None` when the year isn't stored (or no longer validates, which makes
/// the server fetch it again).
async fn load_tax_table(conn: &mut SqliteConnection, year: i16) -> Result<Option<TaxTable>> {
    type Row = (u8, String, i64, Option<i64>, i64, i64, i64, i64, i64, i64);
    let rows: Vec<Row> = sqlx::query_as(
        "SELECT table_no, kind, income_from, income_to, col1, col2, col3, col4, col5, col6
         FROM tax_tables WHERE year = ?",
    )
    .bind(year)
    .fetch_all(&mut *conn)
    .await?;
    if rows.is_empty() {
        return Ok(None);
    }
    let rows = rows
        .into_iter()
        .map(
            |(table, kind, from, to, c1, c2, c3, c4, c5, c6)| TaxTableRow {
                table,
                kind: if kind == "amount" {
                    RowKind::Amount
                } else {
                    RowKind::Percent
                },
                from,
                to,
                columns: [c1, c2, c3, c4, c5, c6],
            },
        )
        .collect();
    Ok(TaxTable::validate(year, rows).ok())
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

/// Books a finalized run as a voucher, on or after its pay date, in one
/// transaction with its `PayrollRunBooked`: both, or nothing and no
/// voucher number used up.
pub async fn book_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<BookedVoucher> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (_, payroll, version) = load(&mut tx, company_id, actor).await?;
    let voucher = domain::book_payroll_run(&payroll, payroll_run_id, today)?;
    let recorded =
        doris_ledger::record_voucher_in(&mut tx, company_id, actor, voucher, today).await?;
    let booked = BookedVoucher {
        fiscal_year_start: recorded.fiscal_year_start,
        number: recorded.number,
    };
    let event = domain::booked(payroll_run_id, booked);
    append(&mut tx, company_id, version, &[event], actor).await?;
    tx.commit().await?;
    Ok(booked)
}

/// Backa bokföring: a rättelse of the run's voucher, after which the run
/// is Färdigställd again. The rättelse is dated `today`, but no later than
/// the end of the voucher's fiscal year, which the ledger requires. No
/// payroll event: the status follows from the rättelse.
pub async fn unbook_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    payroll_run_id: Uuid,
    today: Date,
) -> Result<BookedVoucher> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, payroll, _) = load(&mut tx, company_id, actor).await?;
    let voucher = domain::unbook_payroll_run(&payroll, payroll_run_id)?;
    let year_end = company
        .first_fiscal_year
        .containing(voucher.fiscal_year_start)
        .end;
    let correction = doris_ledger::correct_voucher_in(
        &mut tx,
        company_id,
        actor,
        voucher.fiscal_year_start,
        voucher.number,
        today.min(year_end),
        today,
    )
    .await?;
    tx.commit().await?;
    Ok(BookedVoucher {
        fiscal_year_start: correction.fiscal_year_start,
        number: correction.number,
    })
}

/// One period in the AGI list.
#[derive(Debug, Clone, PartialEq)]
pub struct AgiMonthSummary {
    pub period: Period,
    /// Kronor, all individuppgifter.
    pub gross: i64,
    pub tax_sum: i64,
    pub fee_sum: i64,
    pub status: AgiStatus,
    /// When the latest submission was recorded, if any.
    pub submitted_at: Option<String>,
}

pub async fn agi_contact(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Option<AgiContact>> {
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    Ok(payroll.agi_contact)
}

pub async fn set_agi_contact(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    name: &str,
    phone: &str,
    email: &str,
) -> Result<()> {
    let contact = AgiContact::parse(name, phone, email)?;
    change(pool, company_id, actor, |payroll| {
        Ok(agi::set_agi_contact(payroll, contact))
    })
    .await
}

/// Every period with booked pay or a submission, newest first.
pub async fn agi_months(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<AgiMonthSummary>> {
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    let submitted: Vec<(u32, String)> = sqlx::query_as(
        "SELECT period, MAX(submitted_at) FROM agi_submissions WHERE company_id = ? GROUP BY period",
    )
    .bind(company_id.to_string())
    .fetch_all(&mut *conn)
    .await?;
    let submitted: HashMap<u32, String> = submitted.into_iter().collect();
    Ok(agi::agi_periods(&payroll)
        .into_iter()
        .map(|period| {
            let month = agi::agi_month(&payroll, period);
            AgiMonthSummary {
                period,
                gross: month.lines.iter().map(|(l, _)| l.gross).sum(),
                tax_sum: month.tax_sum,
                fee_sum: month.fee_sum,
                status: month.status,
                submitted_at: submitted.get(&period.get()).cloned(),
            }
        })
        .collect())
}

pub async fn agi_month(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period: Period,
) -> Result<AgiMonth> {
    let mut conn = pool.acquire().await?;
    let (_, payroll, _) = load(&mut conn, company_id, actor).await?;
    Ok(agi::agi_month(&payroll, period))
}

/// The month's file for Skatteverket: its name, the XML and the month's
/// fingerprint (to mark it submitted). It carries personnummer, which is
/// its purpose; it goes only to a member.
pub async fn agi_file(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period: Period,
    created: jiff::civil::DateTime,
) -> Result<(String, String, String)> {
    let mut conn = pool.acquire().await?;
    let (company, payroll, _) = load(&mut conn, company_id, actor).await?;
    let contact = payroll
        .agi_contact
        .clone()
        .ok_or(DomainError::AgiContactMissing)?;
    let month = agi::agi_month(&payroll, period);
    if month.lines.is_empty() && month.removed.is_empty() {
        return Err(DomainError::AgiPeriodEmpty.into());
    }
    let id = agi::employer_id(&company.org_nr, created.year());
    // Every employee, so removed ones have their personnummer too.
    let personal_ids: HashMap<Uuid, String> = payroll
        .employees
        .iter()
        .map(|e| (e.id, e.personal_identity_number.as_str().to_owned()))
        .collect();
    let xml = agi::agi_xml(&month, &id, &contact, &personal_ids, created);
    Ok((format!("AGI_{id}_{period}.xml"), xml, month.fingerprint()))
}

pub async fn submit_agi_month(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    period: Period,
    fingerprint: &str,
) -> Result<()> {
    change(pool, company_id, actor, |payroll| {
        Ok(vec![agi::submit_agi_month(payroll, period, fingerprint)?])
    })
    .await
}
