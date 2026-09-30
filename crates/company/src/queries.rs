//! Read-only views over the company projections.

use crate::Result;
use sqlx::SqlitePool;
use uuid::Uuid;

#[derive(Debug, Clone, PartialEq)]
pub struct CompanySummary {
    pub id: Uuid,
    /// 10 digits, no hyphen.
    pub org_nr: String,
    pub name: String,
}

/// The companies `user_id` is a member of, by name.
pub async fn list_companies(pool: &SqlitePool, user_id: Uuid) -> Result<Vec<CompanySummary>> {
    let rows: Vec<(String, String, String)> = sqlx::query_as(
        "SELECT c.company_id, c.org_nr, c.name
         FROM companies c JOIN company_members m ON m.company_id = c.company_id
         WHERE m.user_id = ? ORDER BY lower(c.name), c.org_nr",
    )
    .bind(user_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(id, org_nr, name)| CompanySummary {
            id: id.parse().expect("company_id is a uuid"),
            org_nr,
            name,
        })
        .collect())
}
