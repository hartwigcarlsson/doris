//! Read-only views over the ledger projections. Each checks membership first.

use crate::Result;
use crate::domain::{Account, AccountName, AccountNumber, Chart};
use sqlx::SqlitePool;
use uuid::Uuid;

/// The company's chart, by number. Before its first change that is the
/// built-in BAS selection, which is not stored until then.
pub async fn list_accounts(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Account>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(i64, String, bool)> = sqlx::query_as(
        "SELECT number, name, active FROM accounts WHERE company_id = ? ORDER BY number",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    if rows.is_empty() {
        return Ok(Chart::from_events(&[crate::domain::seed_chart()])
            .accounts()
            .cloned()
            .collect());
    }
    Ok(rows
        .into_iter()
        .map(|(number, name, active)| Account {
            number: AccountNumber::parse(number as u32).expect("projected numbers are valid"),
            name: AccountName::parse(&name).expect("projected names are valid"),
            active,
        })
        .collect())
}
