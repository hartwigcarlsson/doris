//! Customers and suppliers (kunder och leverantörer) of a company,
//! event-sourced into SQLite. Fakturor will join them here.
//!
//! Every write runs in one IMMEDIATE transaction: check membership, load
//! the register, decide, append, project. A number is decided inside that
//! transaction, so concurrent writers never share one.

pub mod domain;
mod projections;
pub mod vat;

use domain::{
    Change, CustomerDetails, CustomerEvent, CustomerForm, DomainError, Party, PartyDetails,
    Register, SupplierDetails, SupplierEvent, SupplierForm,
};
use doris_eventstore::{Metadata, NewEvent};
use projections::{CUSTOMERS, SUPPLIERS, Table};
use serde::Serialize;
use serde::de::DeserializeOwned;
use sqlx::{SqliteConnection, SqlitePool};
use uuid::Uuid;

pub use projections::rebuild_projections;

const CUSTOMERS_STREAM: &str = "customers-";
const SUPPLIERS_STREAM: &str = "suppliers-";
const SCHEMA_VERSION: i64 = 1;

pub type Customer = Party<CustomerDetails>;
pub type Supplier = Party<SupplierDetails>;

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

pub async fn list_customers(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Customer>> {
    list(pool, &CUSTOMERS, company_id, actor).await
}

pub async fn add_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &CustomerForm<'_>,
) -> Result<u32> {
    let details = CustomerDetails::parse(form)?;
    let changes = change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_customer(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &CustomerForm<'_>,
) -> Result<()> {
    let details = CustomerDetails::parse(form)?;
    change::<_, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_customer_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<CustomerDetails, CustomerEvent>(pool, CUSTOMERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

pub async fn list_suppliers(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Supplier>> {
    list(pool, &SUPPLIERS, company_id, actor).await
}

pub async fn add_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    form: &SupplierForm<'_>,
) -> Result<u32> {
    let details = SupplierDetails::parse(form)?;
    let changes = change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::add(r, details)
    })
    .await?;
    Ok(changes[0].number())
}

pub async fn update_supplier(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    form: &SupplierForm<'_>,
) -> Result<()> {
    let details = SupplierDetails::parse(form)?;
    change::<_, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::update(r, number, details)
    })
    .await?;
    Ok(())
}

pub async fn set_supplier_active(
    pool: &SqlitePool,
    company_id: Uuid,
    actor: Uuid,
    number: u32,
    active: bool,
) -> Result<()> {
    change::<SupplierDetails, SupplierEvent>(pool, SUPPLIERS_STREAM, company_id, actor, |r| {
        domain::set_active(r, number, active)
    })
    .await?;
    Ok(())
}

// ponytail: no pagination; add it when a company has thousands of parties.
async fn list<D: DeserializeOwned>(
    pool: &SqlitePool,
    table: &Table,
    company_id: Uuid,
    actor: Uuid,
) -> Result<Vec<Party<D>>> {
    doris_company::get_company(pool, company_id, actor).await?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(table.list)
        .bind(company_id.to_string())
        .fetch_all(pool)
        .await?;
    rows.into_iter()
        .map(|(number, details, active)| {
            Ok(Party {
                number: u32::try_from(number).expect("projected numbers are u32"),
                details: serde_json::from_str(&details)?,
                active,
            })
        })
        .collect()
}

/// Loads one register's stream, decides, appends and projects, all in one
/// IMMEDIATE transaction. Returns the changes that were recorded.
async fn change<D, E>(
    pool: &SqlitePool,
    stream_prefix: &str,
    company_id: Uuid,
    actor: Uuid,
    decide: impl FnOnce(&Register<D>) -> Result<Vec<Change<D>>, DomainError>,
) -> Result<Vec<Change<D>>>
where
    D: PartyDetails,
    E: Serialize + DeserializeOwned + From<Change<D>> + Into<Change<D>>,
{
    let mut tx = doris_eventstore::begin(pool).await?;
    doris_company::get_company_in(&mut tx, company_id, actor).await?;
    let stream = format!("{stream_prefix}{company_id}");
    let version = doris_eventstore::stream_version(&mut tx, &stream).await?;
    let history = doris_eventstore::load(&mut tx, &stream)
        .await?
        .iter()
        .map(|e| e.decode::<E>().map(Into::into))
        .collect::<Result<Vec<Change<D>>, _>>()?;
    let changes = decide(&Register::from_changes(history))?;
    let events: Vec<E> = changes.iter().cloned().map(E::from).collect();
    append(&mut tx, &stream, version, &events, actor).await?;
    tx.commit().await?;
    Ok(changes)
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
