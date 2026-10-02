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

use domain::{AccountName, AccountNumber, Chart, ChartEvent, DomainError};
use doris_company::domain::Company;
use doris_eventstore::{Metadata, NewEvent};
use serde::Serialize;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::list_accounts;

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
