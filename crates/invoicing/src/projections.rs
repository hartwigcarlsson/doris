//! Read models for the registers. Updated in the same transaction as the
//! append; rebuildable from the event log.

use crate::domain::{Change, CustomerEvent, SupplierEvent};
use crate::supplier_invoices::{SupplierInvoice, SupplierInvoiceEvent};
use crate::{CUSTOMERS_STREAM, SUPPLIER_INVOICES_STREAM, SUPPLIERS_STREAM};
use doris_eventstore::RecordedEvent;
use serde::Serialize;
use sqlx::SqliteConnection;

/// One projection table's statements. Static SQL, since sqlx only takes
/// static strings.
pub(crate) struct Table {
    insert: &'static str,
    update: &'static str,
    set_active: &'static str,
    pub(crate) list: &'static str,
    clear: &'static str,
}

pub(crate) const CUSTOMERS: Table = Table {
    insert: "INSERT INTO customers (company_id, number, details, active) VALUES (?, ?, ?, 1)",
    update: "UPDATE customers SET details = ? WHERE company_id = ? AND number = ?",
    set_active: "UPDATE customers SET active = ? WHERE company_id = ? AND number = ?",
    list: "SELECT number, details, active FROM customers WHERE company_id = ? ORDER BY number",
    clear: "DELETE FROM customers",
};

pub(crate) const SUPPLIERS: Table = Table {
    insert: "INSERT INTO suppliers (company_id, number, details, active) VALUES (?, ?, ?, 1)",
    update: "UPDATE suppliers SET details = ? WHERE company_id = ? AND number = ?",
    set_active: "UPDATE suppliers SET active = ? WHERE company_id = ? AND number = ?",
    list: "SELECT number, details, active FROM suppliers WHERE company_id = ? ORDER BY number",
    clear: "DELETE FROM suppliers",
};

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(company_id) = event.stream_id.strip_prefix(CUSTOMERS_STREAM) {
        let change: Change<_> = event.decode::<CustomerEvent>()?.into();
        return apply_change(conn, &CUSTOMERS, company_id, change).await;
    }
    if let Some(company_id) = event.stream_id.strip_prefix(SUPPLIERS_STREAM) {
        let change: Change<_> = event.decode::<SupplierEvent>()?.into();
        return apply_change(conn, &SUPPLIERS, company_id, change).await;
    }
    if let Some(company_id) = event.stream_id.strip_prefix(SUPPLIER_INVOICES_STREAM) {
        return apply_supplier_invoice(conn, company_id, event.decode()?).await;
    }
    Ok(())
}

async fn apply_supplier_invoice(
    conn: &mut SqliteConnection,
    company_id: &str,
    event: SupplierInvoiceEvent,
) -> crate::Result<()> {
    if let SupplierInvoiceEvent::SupplierInvoiceRegistered {
        number,
        invoice,
        attachments,
        voucher,
    } = event
    {
        let invoice = SupplierInvoice::registered(number, invoice, attachments, voucher);
        sqlx::query(
            "INSERT INTO supplier_invoices
             (company_id, number, supplier_number, invoice_number, status, details)
             VALUES (?, ?, ?, ?, ?, ?)",
        )
        .bind(company_id)
        .bind(number)
        .bind(invoice.invoice.supplier.number)
        .bind(invoice.invoice.invoice_number.as_str())
        .bind(invoice.status_code())
        .bind(serde_json::to_string(&invoice)?)
        .execute(&mut *conn)
        .await?;
        return Ok(());
    }
    let details: String = sqlx::query_scalar(
        "SELECT details FROM supplier_invoices WHERE company_id = ? AND number = ?",
    )
    .bind(company_id)
    .bind(event.number())
    .fetch_one(&mut *conn)
    .await?;
    let mut invoice: SupplierInvoice = serde_json::from_str(&details)?;
    invoice.apply(&event);
    sqlx::query(
        "UPDATE supplier_invoices SET status = ?, details = ? WHERE company_id = ? AND number = ?",
    )
    .bind(invoice.status_code())
    .bind(serde_json::to_string(&invoice)?)
    .bind(company_id)
    .bind(event.number())
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn apply_change<D: Serialize>(
    conn: &mut SqliteConnection,
    table: &Table,
    company_id: &str,
    change: Change<D>,
) -> crate::Result<()> {
    let query = match change {
        Change::Added { number, details } => sqlx::query(table.insert)
            .bind(company_id)
            .bind(number)
            .bind(serde_json::to_string(&details)?),
        Change::Updated { number, details } => sqlx::query(table.update)
            .bind(serde_json::to_string(&details)?)
            .bind(company_id)
            .bind(number),
        Change::Deactivated { number } => sqlx::query(table.set_active)
            .bind(false)
            .bind(company_id)
            .bind(number),
        Change::Reactivated { number } => sqlx::query(table.set_active)
            .bind(true)
            .bind(company_id)
            .bind(number),
    };
    query.execute(&mut *conn).await?;
    Ok(())
}

pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for table in [&CUSTOMERS, &SUPPLIERS] {
        sqlx::query(table.clear).execute(&mut *tx).await?;
    }
    sqlx::query("DELETE FROM supplier_invoices")
        .execute(&mut *tx)
        .await?;
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
