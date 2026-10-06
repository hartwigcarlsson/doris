//! Companies a user keeps the books for, event-sourced into SQLite.
//!
//! Every write runs in one IMMEDIATE transaction: load state, decide, append,
//! project. A UNIQUE violation in a projection rolls the whole write back.

pub mod domain;
mod projections;
mod queries;

use domain::{
    AccountingMethod, Address, Company, CompanyEvent, CompanyName, DomainError, FiscalYear,
    LegalForm, OrgNr, RegisterCompany,
};
use doris_eventstore::{Metadata, NewEvent};
use jiff::civil::Date;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;
pub use queries::{CompanySummary, list_companies};

const COMPANY_STREAM: &str = "company-";
const SCHEMA_VERSION: i64 = 1;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error(transparent)]
    Domain(#[from] DomainError),
    #[error("already exists")]
    AlreadyExists,
    /// No such company, or the user is not a member: callers can't tell which.
    #[error("company not found")]
    NotFound,
    #[error(transparent)]
    Store(doris_eventstore::Error),
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl From<doris_eventstore::Error> for Error {
    fn from(err: doris_eventstore::Error) -> Self {
        match err {
            doris_eventstore::Error::Db(sqlx::Error::Database(db)) if db.is_unique_violation() => {
                Error::AlreadyExists
            }
            other => Error::Store(other),
        }
    }
}

impl From<serde_json::Error> for Error {
    fn from(err: serde_json::Error) -> Self {
        Error::Store(err.into())
    }
}

impl From<sqlx::Error> for Error {
    fn from(err: sqlx::Error) -> Self {
        doris_eventstore::Error::from(err).into()
    }
}

/// Unvalidated input for a new company, as it arrives from the API.
#[derive(Debug, Clone)]
pub struct NewCompany<'a> {
    pub org_nr: &'a str,
    pub name: &'a str,
    pub legal_form: LegalForm,
    pub street: &'a str,
    pub postal_code: &'a str,
    pub city: &'a str,
    pub fiscal_year_start: Date,
    pub fiscal_year_end: Date,
    pub accounting_method: AccountingMethod,
}

/// Registers a company with `created_by` as its first member.
pub async fn register_company(
    pool: &SqlitePool,
    created_by: Uuid,
    input: NewCompany<'_>,
) -> Result<Uuid> {
    let cmd = RegisterCompany {
        company_id: Uuid::new_v4(),
        org_nr: OrgNr::parse(input.org_nr)?,
        name: CompanyName::parse(input.name)?,
        legal_form: input.legal_form,
        address: Address::parse(input.street, input.postal_code, input.city)?,
        first_fiscal_year: FiscalYear::first(
            input.fiscal_year_start,
            input.fiscal_year_end,
            input.legal_form,
        )?,
        accounting_method: input.accounting_method,
    };
    let id = cmd.company_id;
    let events = domain::register_company(cmd, created_by);
    let mut tx = doris_eventstore::begin(pool).await?;
    commit(&mut tx, &company_stream(id), 0, &events, created_by).await?;
    tx.commit().await?;
    Ok(id)
}

pub async fn add_member(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    user_id: Uuid,
) -> Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    let (company, version) = load_company(&mut tx, company_id)
        .await?
        .ok_or(Error::NotFound)?;
    let events = domain::add_member(&company, actor, user_id)?;
    commit(
        &mut tx,
        &company_stream(company_id),
        version,
        &events,
        actor,
    )
    .await?;
    tx.commit().await?;
    Ok(())
}

/// The company, if `user_id` is a member of it.
pub async fn get_company(pool: &SqlitePool, company_id: Uuid, user_id: Uuid) -> Result<Company> {
    let mut conn = pool.acquire().await?;
    get_company_in(&mut conn, company_id, user_id).await
}

/// [`get_company`] on the caller's connection, e.g. inside another crate's
/// write transaction.
pub async fn get_company_in(
    conn: &mut SqliteConnection,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Company> {
    match load_company(conn, company_id).await? {
        Some((company, _)) if company.is_member(user_id) => Ok(company),
        _ => Err(Error::NotFound),
    }
}

fn company_stream(id: Uuid) -> String {
    format!("{COMPANY_STREAM}{id}")
}

async fn load_company(conn: &mut SqliteConnection, id: Uuid) -> Result<Option<(Company, i64)>> {
    let recorded = doris_eventstore::load(conn, &company_stream(id)).await?;
    let version = recorded.last().map_or(0, |e| e.stream_version);
    let events = recorded
        .iter()
        .map(|e| e.decode::<CompanyEvent>())
        .collect::<Result<Vec<_>, _>>()?;
    Ok(Company::from_events(&events).map(|company| (company, version)))
}

/// Appends events and updates projections within the caller's transaction.
async fn commit(
    conn: &mut SqliteConnection,
    stream: &str,
    expected_version: i64,
    events: &[CompanyEvent],
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
        ..Default::default()
    };
    let recorded =
        doris_eventstore::append(conn, stream, expected_version, &new_events, &metadata).await?;
    for event in &recorded {
        projections::apply(conn, event).await?;
    }
    Ok(())
}
