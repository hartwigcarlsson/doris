//! Read-only views over the ledger projections. Each checks membership first.

use crate::Result;
use crate::domain::{Account, AccountName, AccountNumber, Chart, Voucher, VoucherLine};
use doris_company::domain::FiscalYear;
use jiff::civil::Date;
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

/// The company's räkenskapsår from the first up to the one containing
/// `today`, newest first.
pub async fn list_fiscal_years(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    today: Date,
) -> Result<Vec<FiscalYear>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let mut years = vec![company.first_fiscal_year];
    while let Some(last) = years.last().copied()
        && last.end < today
    {
        years.push(last.next());
    }
    years.reverse();
    Ok(years)
}

/// The grundbok for one fiscal year: every voucher with its lines, by number.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn list_vouchers(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
) -> Result<Vec<Voucher>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let (company_id, fiscal_year_start) = (company_id.to_string(), fiscal_year_start.to_string());
    type Head = (u32, String, String, Option<u32>, Option<u32>);
    let heads: Vec<Head> = sqlx::query_as(
        "SELECT number, date, text, corrects, corrected_by FROM vouchers
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(pool)
    .await?;
    let lines: Vec<(u32, u32, i64, i64)> = sqlx::query_as(
        "SELECT number, account, debit, credit FROM voucher_lines
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number, line_no",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(pool)
    .await?;
    let mut vouchers: Vec<Voucher> = heads
        .into_iter()
        .map(|(number, date, text, corrects, corrected_by)| Voucher {
            number,
            date: date.parse().expect("projected dates are valid"),
            text,
            lines: Vec::new(),
            corrects,
            corrected_by,
        })
        .collect();
    for (number, account, debit, credit) in lines {
        // Numbers run 1..=n (the trigger guarantees it), so number - 1 is the index.
        vouchers[number as usize - 1]
            .lines
            .push(VoucherLine::new(account, debit, credit).expect("projected accounts are valid"));
    }
    Ok(vouchers)
}
