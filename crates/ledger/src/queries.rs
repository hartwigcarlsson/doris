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
    // One read transaction: heads and lines from the same snapshot (WAL).
    let mut tx = pool.begin().await?;
    let heads: Vec<Head> = sqlx::query_as(
        "SELECT number, date, text, corrects, corrected_by FROM vouchers
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(&mut *tx)
    .await?;
    let lines: Vec<(u32, u32, i64, i64)> = sqlx::query_as(
        "SELECT number, account, debit, credit FROM voucher_lines
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number, line_no",
    )
    .bind(&company_id)
    .bind(&fiscal_year_start)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
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
    attach_lines(&mut vouchers, lines);
    Ok(vouchers)
}

/// Puts each `(number, account, debit, credit)` line on its voucher.
fn attach_lines(vouchers: &mut [Voucher], lines: Vec<(u32, u32, i64, i64)>) {
    for (number, account, debit, credit) in lines {
        // Numbers run 1..=n (the trigger guarantees it), so number - 1 is the
        // index. A line whose head is missing is skipped, never a panic.
        if let Some(voucher) = (number as usize)
            .checked_sub(1)
            .and_then(|i| vouchers.get_mut(i))
            .filter(|v| v.number == number)
        {
            voucher.lines.push(
                VoucherLine::new(account, debit, credit).expect("projected accounts are valid"),
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_line_without_its_voucher_is_skipped() {
        let mut vouchers = vec![Voucher {
            number: 1,
            date: jiff::civil::date(2026, 1, 1),
            text: "Kassa".into(),
            lines: Vec::new(),
            corrects: None,
            corrected_by: None,
        }];
        // Voucher 2 was committed after the heads were read.
        attach_lines(
            &mut vouchers,
            vec![(1, 1930, 100, 0), (2, 1930, 50, 0), (0, 1930, 1, 0)],
        );
        assert_eq!(vouchers.len(), 1);
        assert_eq!(
            vouchers[0].lines,
            vec![VoucherLine::new(1930, 100, 0).unwrap()]
        );
    }
}
