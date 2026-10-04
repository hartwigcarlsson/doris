//! Read models for employees and payroll runs. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::PAYROLL_STREAM;
use crate::domain::PayrollEvent;
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
        // Projected by Task 6 of the plan, which replaces this arm.
        PayrollEvent::PayrollRunCreated { .. }
        | PayrollEvent::PayrollRunUpdated { .. }
        | PayrollEvent::PayrollRunFinalized { .. }
        | PayrollEvent::PayrollRunReopened { .. }
        | PayrollEvent::PayrollRunBooked { .. } => {}
    }
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
