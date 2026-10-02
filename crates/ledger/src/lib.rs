//! The chart of accounts and the vouchers (verifikationer) of a company,
//! event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, load
//! state, decide, append, project. A voucher's number is decided inside that
//! transaction, so a write that fails or rolls back uses up no number.

mod bas;
pub mod domain;
mod projections;
mod queries;

use domain::{
    AccountName, AccountNumber, Chart, ChartEvent, DomainError, Ledger, LedgerEvent, RecordVoucher,
};
use doris_company::domain::{Company, FiscalYear};
use doris_eventstore::{Metadata, NewEvent};
use jiff::civil::Date;
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::{account_ledger, list_accounts, list_fiscal_years, list_vouchers, trial_balance};

const ACCOUNTS_STREAM: &str = "accounts-";
const LEDGER_STREAM: &str = "ledger-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    /// A sum outgrew `i64`; no real ledger gets there.
    #[error("amount overflow")]
    Overflow,
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

pub async fn add_account(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    name: &str,
) -> Result<()> {
    let (number, name) = (AccountNumber::parse(number)?, AccountName::parse(name)?);
    change_chart(pool, company_id, actor, |chart| {
        domain::add_account(chart, number, name)
    })
    .await
}

pub async fn rename_account(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    name: &str,
) -> Result<()> {
    let (number, name) = (AccountNumber::parse(number)?, AccountName::parse(name)?);
    change_chart(pool, company_id, actor, |chart| {
        domain::rename_account(chart, number, name)
    })
    .await
}

pub async fn set_account_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    let number = AccountNumber::parse(number)?;
    change_chart(pool, company_id, actor, |chart| {
        domain::set_account_active(chart, number, active)
    })
    .await
}

async fn change_chart(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Chart) -> Result<Vec<ChartEvent>, DomainError>,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    member_company(&mut tx, company_id, actor).await?;
    let chart = seeded_chart(&mut tx, company_id, actor).await?;
    let version = doris_eventstore::stream_version(&mut tx, &accounts_stream(company_id)).await?;
    let events = decide(&chart)?;
    append(
        &mut tx,
        &accounts_stream(company_id),
        version,
        &events,
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

async fn member_company(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Company> {
    Ok(doris_company::get_company_in(conn, company_id, user_id).await?)
}

fn accounts_stream(company_id: Uuid) -> String {
    format!("{ACCOUNTS_STREAM}{company_id}")
}

/// The company's chart. A company that has none yet gets the built-in BAS
/// selection appended first, in the caller's transaction.
async fn seeded_chart(conn: &mut SqliteConnection, company_id: Uuid, actor: Uuid) -> Result<Chart> {
    let stream = accounts_stream(company_id);
    let recorded = doris_eventstore::load(conn, &stream).await?;
    if recorded.is_empty() {
        let seed = domain::seed_chart();
        append(conn, &stream, 0, std::slice::from_ref(&seed), actor).await?;
        return Ok(Chart::from_events(&[seed]));
    }
    let events = recorded
        .iter()
        .map(|e| e.decode::<ChartEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Chart::from_events(&events))
}

/// Appends events and updates projections within the caller's transaction.
async fn append<E: Serialize>(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[E],
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
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}

/// Where a voucher landed: its fiscal year and its number in that year.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct VoucherRef {
    pub fiscal_year_start: Date,
    pub number: u32,
}

pub async fn record_voucher(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    today: Date,
) -> Result<VoucherRef> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = record_voucher_in(&mut tx, company_id, actor, cmd, today).await?;
    tx.commit().await?;
    Ok(voucher)
}

/// [`record_voucher`] in the caller's transaction, which must be IMMEDIATE
/// (see [`doris_eventstore::begin`]). Nothing is kept, and no number is used
/// up, unless the caller commits.
pub async fn record_voucher_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    cmd: RecordVoucher,
    today: Date,
) -> Result<VoucherRef> {
    let company = member_company(conn, company_id, actor).await?;
    let fiscal_year = domain::fiscal_year_for(company.first_fiscal_year, cmd.date, today)?;
    let chart = seeded_chart(conn, company_id, actor).await?;
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let event = domain::record_voucher(&ledger, &chart, cmd)?;
    commit_voucher(conn, company_id, fiscal_year, version, event, actor).await
}

pub async fn correct_voucher(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    date: Date,
    today: Date,
) -> Result<VoucherRef> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let voucher = correct_voucher_in(
        &mut tx,
        company_id,
        actor,
        fiscal_year_start,
        number,
        date,
        today,
    )
    .await?;
    tx.commit().await?;
    Ok(voucher)
}

/// [`correct_voucher`] in the caller's IMMEDIATE transaction.
pub async fn correct_voucher_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    actor: Uuid,
    fiscal_year_start: Date,
    number: u32,
    date: Date,
    today: Date,
) -> Result<VoucherRef> {
    let company = member_company(conn, company_id, actor).await?;
    // A year that starts after today has no vouchers; checked first so a
    // far-future start never steps fiscal years past the date limits.
    if fiscal_year_start > today {
        return Err(DomainError::VoucherNotFound.into());
    }
    let fiscal_year = company.first_fiscal_year.containing(fiscal_year_start);
    if fiscal_year.start != fiscal_year_start {
        return Err(DomainError::VoucherNotFound.into());
    }
    let (ledger, version) = load_ledger(conn, company_id, fiscal_year).await?;
    let event = domain::correct_voucher(&ledger, number, date, today)?;
    commit_voucher(conn, company_id, fiscal_year, version, event, actor).await
}

async fn commit_voucher(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    fiscal_year: FiscalYear,
    version: i64,
    event: LedgerEvent,
    actor: Uuid,
) -> Result<VoucherRef> {
    let LedgerEvent::VoucherRecorded { number, .. } = event;
    let stream = ledger_stream(company_id, fiscal_year.start);
    append(conn, &stream, version, &[event], actor).await?;
    Ok(VoucherRef {
        fiscal_year_start: fiscal_year.start,
        number,
    })
}

fn ledger_stream(company_id: Uuid, fiscal_year_start: Date) -> String {
    format!("{LEDGER_STREAM}{company_id}-{fiscal_year_start}")
}

/// The ledger stream id's parts: company id and fiscal year start.
fn parse_ledger_stream(stream_id: &str) -> Option<(&str, &str)> {
    let rest = stream_id.strip_prefix(LEDGER_STREAM)?;
    // A uuid is 36 characters, followed by '-' and the YYYY-MM-DD start.
    Some((rest.get(..36)?, rest.get(37..)?))
}

async fn load_ledger(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    fiscal_year: FiscalYear,
) -> Result<(Ledger, i64)> {
    let recorded =
        doris_eventstore::load(conn, &ledger_stream(company_id, fiscal_year.start)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<LedgerEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok((Ledger::from_events(fiscal_year, &events), version))
}
