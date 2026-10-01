//! Read models for companies and their members. Updated in the same
//! transaction as the append; rebuildable from the event log.

use crate::domain::CompanyEvent;
use doris_eventstore::RecordedEvent;
use sqlx::SqliteConnection;

pub(crate) async fn apply(conn: &mut SqliteConnection, event: &RecordedEvent) -> crate::Result<()> {
    let Some(company_id) = event.stream_id.strip_prefix(crate::COMPANY_STREAM) else {
        return Ok(());
    };
    let at = &event.recorded_at;
    match event.decode::<CompanyEvent>()? {
        CompanyEvent::CompanyRegistered {
            org_nr,
            name,
            legal_form,
            address,
            first_fiscal_year,
            accounting_method,
            ..
        } => {
            sqlx::query(
                "INSERT INTO companies (company_id, org_nr, name, legal_form, street, postal_code,
                     city, first_fiscal_year_start, first_fiscal_year_end, accounting_method,
                     registered_at)
                 VALUES (?, ?, ?, ?, ?, ?, ?, ?, ?, ?, ?)",
            )
            .bind(company_id)
            .bind(org_nr.as_str())
            .bind(name.as_str())
            .bind(legal_form.as_str())
            .bind(address.street)
            .bind(address.postal_code)
            .bind(address.city)
            .bind(first_fiscal_year.start.to_string())
            .bind(first_fiscal_year.end.to_string())
            .bind(accounting_method.as_str())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
        CompanyEvent::MemberAdded { user_id, .. } => {
            sqlx::query(
                "INSERT INTO company_members (company_id, user_id, added_at) VALUES (?, ?, ?)",
            )
            .bind(company_id)
            .bind(user_id.to_string())
            .bind(at)
            .execute(&mut *conn)
            .await?;
        }
    }
    Ok(())
}

/// Empties the company projections and replays the whole event log.
pub async fn rebuild_projections(pool: &sqlx::SqlitePool) -> crate::Result<()> {
    let mut tx = doris_eventstore::begin(pool).await?;
    for statement in ["DELETE FROM company_members", "DELETE FROM companies"] {
        sqlx::query(statement).execute(&mut *tx).await?;
    }
    for event in doris_eventstore::read_all(&mut tx, 0).await? {
        apply(&mut tx, &event).await?;
    }
    tx.commit().await?;
    Ok(())
}
