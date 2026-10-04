//! Reads over the payroll projections. Every read checks membership first.

use crate::Result;
use crate::domain::{
    self, BookedVoucher, DomainError, Employee, EmployeeName, PayrollRunLine, PayrollRunStatus,
    PersonalIdentityNumber, SalaryAccount,
};
use doris_ledger::domain::VoucherLine;
use jiff::civil::Date;
use sqlx::SqlitePool;
use std::collections::HashMap;
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
                tax: None, // Task 3 reads it from the projection.
            })
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRunView {
    pub id: Uuid,
    pub pay_date: Date,
    pub text: String,
    pub status: PayrollRunStatus,
    pub lines: Vec<PayrollRunLineView>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct PayrollRunLineView {
    pub employee_id: Uuid,
    pub employee_name: String,
    pub gross: i64,
    pub tax: i64,
    /// Set while the run is finalized or booked.
    pub locked: Option<PayrollRunLine>,
}

impl PayrollRunView {
    /// The voucher the run books; empty while it is open.
    pub fn voucher_lines(&self) -> Vec<VoucherLine> {
        let locked: Option<Vec<PayrollRunLine>> = self.lines.iter().map(|l| l.locked).collect();
        locked.map_or_else(Vec::new, |lines| domain::voucher_lines(&lines))
    }
}

/// Every payroll run, newest pay date first. A run is booked while its
/// latest booking's voucher has no rättelse (the same rule as
/// `Payroll::status`).
pub async fn list_payroll_runs(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<PayrollRunView>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let company = company_id.to_string();
    // ponytail: every run of the company in one list; filter by year or
    // page when a company has several years of runs.
    type Head = (
        String,
        String,
        String,
        bool,
        Option<String>,
        Option<u32>,
        bool,
    );
    // One read transaction: heads and lines from the same snapshot (WAL).
    let mut tx = pool.begin().await?;
    let heads: Vec<Head> = sqlx::query_as(
        "SELECT r.payroll_run_id, r.pay_date, r.text, r.finalized,
                b.fiscal_year_start, b.voucher_number,
                EXISTS (SELECT 1 FROM vouchers v
                        WHERE v.company_id = b.company_id
                          AND v.fiscal_year_start = b.fiscal_year_start
                          AND v.corrects = b.voucher_number)
         FROM payroll_runs r
         LEFT JOIN payroll_run_bookings b ON b.rowid = (
             SELECT MAX(x.rowid) FROM payroll_run_bookings x
             WHERE x.company_id = r.company_id AND x.payroll_run_id = r.payroll_run_id)
         WHERE r.company_id = ?
         ORDER BY r.pay_date DESC, r.rowid DESC",
    )
    .bind(&company)
    .fetch_all(&mut *tx)
    .await?;
    type Line = (
        String,
        String,
        String,
        i64,
        i64,
        Option<u32>,
        Option<u32>,
        Option<i64>,
        Option<i64>,
    );
    let rows: Vec<Line> = sqlx::query_as(
        "SELECT l.payroll_run_id, l.employee_id, e.name, l.gross, l.tax,
                l.salary_account, l.fee_rate, l.fee, l.net
         FROM payroll_run_lines l
         JOIN employees e ON e.company_id = l.company_id AND e.employee_id = l.employee_id
         WHERE l.company_id = ?
         ORDER BY e.name, l.employee_id",
    )
    .bind(&company)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;

    let mut lines = HashMap::<String, Vec<PayrollRunLineView>>::new();
    for (run, employee, name, gross, tax, account, fee_rate, fee, net) in rows {
        let employee_id: Uuid = employee.parse().expect("stored uuids parse");
        let locked = match (account, fee_rate, fee, net) {
            (Some(account), Some(fee_rate), Some(fee), Some(net)) => Some(PayrollRunLine {
                employee_id,
                salary_account: SalaryAccount::parse(account)?,
                gross,
                tax,
                fee_rate,
                fee,
                net,
                tax_basis: crate::tax::TaxBasis::Manual,
            }),
            _ => None,
        };
        lines.entry(run).or_default().push(PayrollRunLineView {
            employee_id,
            employee_name: name,
            gross,
            tax,
            locked,
        });
    }
    Ok(heads
        .into_iter()
        .map(|(id, pay_date, text, finalized, start, number, reversed)| {
            let status = match (start, number) {
                (Some(start), Some(number)) if !reversed => {
                    PayrollRunStatus::Booked(BookedVoucher {
                        fiscal_year_start: start.parse().expect("stored dates parse"),
                        number,
                    })
                }
                _ if finalized => PayrollRunStatus::Finalized,
                _ => PayrollRunStatus::Open,
            };
            PayrollRunView {
                lines: lines.remove(&id).unwrap_or_default(),
                id: id.parse().expect("stored uuids parse"),
                pay_date: pay_date.parse().expect("stored dates parse"),
                text,
                status,
            }
        })
        .collect())
}

pub async fn get_payroll_run(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    payroll_run_id: Uuid,
) -> Result<PayrollRunView> {
    // ponytail: reads the whole list; a single-run query when lists grow.
    list_payroll_runs(pool, company_id, user_id)
        .await?
        .into_iter()
        .find(|r| r.id == payroll_run_id)
        .ok_or_else(|| DomainError::PayrollRunNotFound.into())
}
