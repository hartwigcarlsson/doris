//! Read models for employees and payroll runs. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::PAYROLL_STREAM;
use crate::domain::{DraftLine, PayrollEvent, PayrollRunLine};
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    let Some(company_id) = event.stream_id.strip_prefix(PAYROLL_STREAM) else {
        return Ok(());
    };
    match event.decode()? {
        PayrollEvent::EmployeeAdded {
            employee_id,
            name,
            personal_identity_number,
            monthly_salary,
            salary_account,
        } => {
            sqlx::query(
                "INSERT INTO employees (company_id, employee_id, name, personal_identity_number,
                     monthly_salary, salary_account, active)
                 VALUES (?, ?, ?, ?, ?, ?, 1)",
            )
            .bind(company_id)
            .bind(employee_id.to_string())
            .bind(name.as_str())
            .bind(personal_identity_number.as_str())
            .bind(monthly_salary)
            .bind(salary_account.get())
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::EmployeeUpdated {
            employee_id,
            name,
            monthly_salary,
            salary_account,
        } => {
            sqlx::query(
                "UPDATE employees SET name = ?, monthly_salary = ?, salary_account = ?
                 WHERE company_id = ? AND employee_id = ?",
            )
            .bind(name.as_str())
            .bind(monthly_salary)
            .bind(salary_account.get())
            .bind(company_id)
            .bind(employee_id.to_string())
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::EmployeeDeactivated { employee_id } => {
            sqlx::query("UPDATE employees SET active = 0 WHERE company_id = ? AND employee_id = ?")
                .bind(company_id)
                .bind(employee_id.to_string())
                .execute(&mut *conn)
                .await?;
        }
        PayrollEvent::PayrollRunCreated {
            payroll_run_id,
            draft,
        } => {
            let run = payroll_run_id.to_string();
            sqlx::query(
                "INSERT INTO payroll_runs (company_id, payroll_run_id, pay_date, text, finalized,
                     updated_at, updated_by)
                 VALUES (?, ?, ?, ?, 0, ?, ?)",
            )
            .bind(company_id)
            .bind(&run)
            .bind(draft.pay_date.to_string())
            .bind(&draft.text)
            .bind(&event.recorded_at)
            .bind(actor(event))
            .execute(&mut *conn)
            .await?;
            insert_draft_lines(conn, company_id, &run, &draft.lines).await?;
        }
        PayrollEvent::PayrollRunUpdated {
            payroll_run_id,
            draft,
        } => {
            let run = payroll_run_id.to_string();
            sqlx::query(
                "UPDATE payroll_runs SET pay_date = ?, text = ?, updated_at = ?, updated_by = ?
                 WHERE company_id = ? AND payroll_run_id = ?",
            )
            .bind(draft.pay_date.to_string())
            .bind(&draft.text)
            .bind(&event.recorded_at)
            .bind(actor(event))
            .bind(company_id)
            .bind(&run)
            .execute(&mut *conn)
            .await?;
            delete_lines(conn, company_id, &run).await?;
            insert_draft_lines(conn, company_id, &run, &draft.lines).await?;
        }
        PayrollEvent::PayrollRunFinalized {
            payroll_run_id,
            lines,
        } => {
            let run = payroll_run_id.to_string();
            set_finalized(conn, company_id, &run, true, event).await?;
            delete_lines(conn, company_id, &run).await?;
            for line in &lines {
                insert_locked_line(conn, company_id, &run, line).await?;
            }
        }
        PayrollEvent::PayrollRunReopened { payroll_run_id } => {
            let run = payroll_run_id.to_string();
            set_finalized(conn, company_id, &run, false, event).await?;
            sqlx::query(
                "UPDATE payroll_run_lines
                 SET salary_account = NULL, fee_rate = NULL, fee = NULL, net = NULL
                 WHERE company_id = ? AND payroll_run_id = ?",
            )
            .bind(company_id)
            .bind(&run)
            .execute(&mut *conn)
            .await?;
        }
        PayrollEvent::PayrollRunBooked {
            payroll_run_id,
            voucher,
        } => {
            sqlx::query(
                "INSERT INTO payroll_run_bookings (company_id, payroll_run_id, fiscal_year_start,
                     voucher_number)
                 VALUES (?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(payroll_run_id.to_string())
            .bind(voucher.fiscal_year_start.to_string())
            .bind(voucher.number)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

fn actor(event: &RecordedEvent) -> &str {
    event.metadata.actor.as_deref().unwrap_or_default()
}

async fn set_finalized(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    finalized: bool,
    event: &RecordedEvent,
) -> crate::Result<()> {
    sqlx::query(
        "UPDATE payroll_runs SET finalized = ?, updated_at = ?, updated_by = ?
         WHERE company_id = ? AND payroll_run_id = ?",
    )
    .bind(finalized)
    .bind(&event.recorded_at)
    .bind(actor(event))
    .bind(company_id)
    .bind(run)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

async fn delete_lines(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
) -> crate::Result<()> {
    sqlx::query("DELETE FROM payroll_run_lines WHERE company_id = ? AND payroll_run_id = ?")
        .bind(company_id)
        .bind(run)
        .execute(&mut *conn)
        .await?;
    Ok(())
}

async fn insert_draft_lines(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    lines: &[DraftLine],
) -> crate::Result<()> {
    for line in lines {
        sqlx::query(
            "INSERT INTO payroll_run_lines (company_id, payroll_run_id, employee_id, gross, tax)
             VALUES (?, ?, ?, ?, ?)",
        )
        .bind(company_id)
        .bind(run)
        .bind(line.employee_id.to_string())
        .bind(line.gross)
        .bind(line.tax)
        .execute(&mut *conn)
        .await?;
    }
    Ok(())
}

async fn insert_locked_line(
    conn: &mut SqliteConnection,
    company_id: &str,
    run: &str,
    line: &PayrollRunLine,
) -> crate::Result<()> {
    sqlx::query(
        "INSERT INTO payroll_run_lines (company_id, payroll_run_id, employee_id, gross, tax,
             salary_account, fee_rate, fee, net)
         VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?)",
    )
    .bind(company_id)
    .bind(run)
    .bind(line.employee_id.to_string())
    .bind(line.gross)
    .bind(line.tax)
    .bind(line.salary_account.get())
    .bind(line.fee_rate)
    .bind(line.fee)
    .bind(line.net)
    .execute(&mut *conn)
    .await?;
    Ok(())
}

/// Empties the payroll projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in [
        "DELETE FROM payroll_run_bookings",
        "DELETE FROM payroll_run_lines",
        "DELETE FROM payroll_runs",
        "DELETE FROM employees",
    ] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
