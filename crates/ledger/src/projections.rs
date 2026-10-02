//! Read models for the chart and the vouchers. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::ACCOUNTS_STREAM;
use crate::domain::{ChartAccount, ChartEvent, LedgerEvent};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    if let Some(company_id) = event.stream_id.strip_prefix(ACCOUNTS_STREAM) {
        return apply_chart(conn, company_id, event.decode()?).await;
    }
    if let Some((company_id, fiscal_year_start)) = crate::parse_ledger_stream(&event.stream_id) {
        return apply_ledger(conn, company_id, fiscal_year_start, event).await;
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

async fn apply_ledger(
    conn: &mut SqliteConnection,
    company_id: &str,
    fiscal_year_start: &str,
    event: &RecordedEvent,
) -> crate::Result<()> {
    match event.decode()? {
        LedgerEvent::VoucherRecorded {
            number,
            date,
            text,
            lines,
            corrects,
        } => {
            sqlx::query(
                "INSERT INTO vouchers (company_id, fiscal_year_start, number, date, text, corrects,
                 recorded_at, recorded_by)
             VALUES (?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(fiscal_year_start)
            .bind(number)
            .bind(date.to_string())
            .bind(&text)
            .bind(corrects)
            .bind(&event.recorded_at)
            .bind(event.metadata.actor.as_deref().unwrap_or_default())
            .execute(&mut *conn)
            .await?;
            for (line_no, line) in (1_i64..).zip(&lines) {
                sqlx::query(
                "INSERT INTO voucher_lines (company_id, fiscal_year_start, number, line_no, account,
                     debit, credit)
                 VALUES (?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(fiscal_year_start)
            .bind(number)
            .bind(line_no)
            .bind(line.account.get())
            .bind(line.debit)
            .bind(line.credit)
            .execute(&mut *conn)
            .await?;
            }
            if let Some(original) = corrects {
                sqlx::query(
                    "UPDATE vouchers SET corrected_by = ?
                 WHERE company_id = ? AND fiscal_year_start = ? AND number = ?",
                )
                .bind(number)
                .bind(company_id)
                .bind(fiscal_year_start)
                .bind(original)
                .execute(&mut *conn)
                .await?;
            }
        }
        // Projected in Task 3.
        LedgerEvent::OpeningBalancesSet { .. }
        | LedgerEvent::FiscalYearClosed { .. }
        | LedgerEvent::FiscalYearReopened { .. } => {}
    }
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
