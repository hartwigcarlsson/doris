//! Reads over the payroll projections. Every read checks membership first.

use crate::Result;
use crate::domain::{Employee, EmployeeName, PersonalIdentityNumber, SalaryAccount};
use sqlx::SqlitePool;
use uuid::Uuid;

/// All employees, inactive too, by name.
pub async fn list_employees(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<Employee>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(String, String, String, i64, u32, bool)> = sqlx::query_as(
        "SELECT employee_id, name, personal_identity_number, monthly_salary, salary_account, active
         FROM employees WHERE company_id = ? ORDER BY name, employee_id",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    rows.into_iter()
        .map(|(id, name, pin, monthly_salary, account, active)| {
            Ok(Employee {
                id: id.parse().expect("stored uuids parse"),
                name: EmployeeName::parse(&name)?,
                personal_identity_number: PersonalIdentityNumber::parse(&pin)?,
                monthly_salary,
                salary_account: SalaryAccount::parse(account)?,
                active,
            })
        })
        .collect()
}
