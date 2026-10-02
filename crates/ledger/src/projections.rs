//! Read models for the chart and the vouchers. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::{ChartAccount, ChartEvent};
use crate::{ACCOUNTS_STREAM, LEDGER_STREAM};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(company_id) = event.stream_id.strip_prefix(ACCOUNTS_STREAM) {
        return apply_chart(conn, company_id, event.decode()?).await;
    }
    if event.stream_id.starts_with(LEDGER_STREAM) {
        return Ok(()); // Task 4
    }
    Ok(())
}

async fn apply_chart(
    conn: &mut SqliteConnection,
    company_id: &str,
    event: ChartEvent,
) -> crate::Result<()> {
    match event {
        ChartEvent::ChartSeeded { accounts } => {
            for ChartAccount { number, name } in accounts {
                insert_account(conn, company_id, number.get(), name.as_str()).await?;
            }
        }
        ChartEvent::AccountAdded { number, name } => {
            insert_account(conn, company_id, number.get(), name.as_str()).await?;
        }
        ChartEvent::AccountRenamed { number, name } => {
            sqlx::query("UPDATE accounts SET name = ? WHERE company_id = ? AND number = ?")
                .bind(name.as_str())
                .bind(company_id)
                .bind(number.get())
                .execute(&mut *conn)
                .await?;
        }
        ChartEvent::AccountDeactivated { number } => {
            set_active(conn, company_id, number.get(), false).await?
        }
        ChartEvent::AccountReactivated { number } => {
            set_active(conn, company_id, number.get(), true).await?
        }
    }
    Ok(())
}

async fn insert_account(
    conn: &mut SqliteConnection,
    company_id: &str,
    number: u16,
    name: &str,
) -> crate::Result<()> {
    sqlx::query("INSERT INTO accounts (company_id, number, name, active) VALUES (?, ?, ?, 1)")
        .bind(company_id)
        .bind(number)
        .bind(name)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn set_active(
    conn: &mut SqliteConnection,
    company_id: &str,
    number: u16,
    active: bool,
) -> crate::Result<()> {
    sqlx::query("UPDATE accounts SET active = ? WHERE company_id = ? AND number = ?")
        .bind(active)
        .bind(company_id)
        .bind(number)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

/// Empties the ledger projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in [
        "DELETE FROM voucher_lines",
        "DELETE FROM vouchers",
        "DELETE FROM accounts",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
