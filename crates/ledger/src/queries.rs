//! Read-only views over the ledger projections. Each checks membership first.

use crate::domain::{
    Account, AccountLedger, AccountName, AccountNumber, Attachment, AttachmentName, Chart,
    ContentType, DomainError, FiscalYearStatus, TrialBalanceRow, Voucher, VoucherLine,
    running_balance,
};
use crate::{Error, Result};
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
/// `today`, newest first, each with whether it is closed.
pub async fn list_fiscal_years(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    today: Date,
) -> Result<Vec<FiscalYearStatus>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let closed: Vec<String> = sqlx::query_scalar(
        "SELECT fiscal_year_start FROM closed_fiscal_years WHERE company_id = ?",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    let mut years = vec![company.first_fiscal_year];
    while let Some(last) = years.last().copied()
        && last.end < today
    {
        years.push(last.next());
    }
    Ok(years
        .into_iter()
        .rev()
        .map(|fiscal_year| FiscalYearStatus {
            closed: closed.contains(&fiscal_year.start.to_string()),
            fiscal_year,
        })
        .collect())
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
    let attachments: Vec<(u32, String, String, String, i64)> = sqlx::query_as(
        "SELECT number, sha256, file_name, content_type, size FROM voucher_attachments
         WHERE company_id = ? AND fiscal_year_start = ? ORDER BY number, position",
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
            attachments: Vec::new(),
        })
        .collect();
    attach_lines(&mut vouchers, lines);
    for (number, sha256, file_name, content_type, size) in attachments {
        if let Some(voucher) = voucher_at(&mut vouchers, number) {
            let attachment = projected_attachment(sha256, &file_name, &content_type, size);
            voucher.attachments.push(attachment);
        }
    }
    Ok(vouchers)
}

/// One underlag and its bytes, found through the company's own voucher: a
/// hash alone never reads another company's file.
pub async fn get_attachment(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    number: u32,
    sha256: &str,
) -> Result<(Attachment, Vec<u8>)> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let row: Option<(String, String, i64, Vec<u8>)> = sqlx::query_as(
        "SELECT a.file_name, a.content_type, a.size, f.data
         FROM voucher_attachments a JOIN attachment_files f ON f.sha256 = a.sha256
         WHERE a.company_id = ? AND a.fiscal_year_start = ? AND a.number = ? AND a.sha256 = ?",
    )
    .bind(company_id.to_string())
    .bind(fiscal_year_start.to_string())
    .bind(number)
    .bind(sha256)
    .fetch_optional(pool)
    .await?;
    let (file_name, content_type, size, data) = row.ok_or(DomainError::AttachmentNotFound)?;
    Ok((
        projected_attachment(sha256.to_owned(), &file_name, &content_type, size),
        data,
    ))
}

/// The saldobalans for one fiscal year, by account number: every account
/// with lines in the year or a non-zero ingående balans. The ingående balans
/// is the first year's opening balances plus every earlier year's lines on
/// accounts 1000–2999.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn trial_balance(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
) -> Result<Vec<TrialBalanceRow>> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    // A year before the first has no opening balances either.
    let has_opening = fiscal_year_start >= company.first_fiscal_year.start;
    let (company_id, start) = (company_id.to_string(), fiscal_year_start.to_string());
    // One statement, so one snapshot. LEFT JOIN keeps an account even if the
    // chart somehow lacked it. SQLite's SUM fails on overflow, never wraps.
    let rows: Vec<(u32, String, i64, i64, i64)> = sqlx::query_as(
        "SELECT t.account, COALESCE(a.name, ''), SUM(t.opening), SUM(t.debit), SUM(t.credit)
         FROM (
             SELECT account, debit - credit AS opening, 0 AS debit, 0 AS credit, 0 AS in_year
             FROM opening_balances WHERE company_id = ? AND ?
             UNION ALL
             SELECT account, debit - credit, 0, 0, 0 FROM voucher_lines
             WHERE company_id = ? AND fiscal_year_start < ? AND account < 3000
             UNION ALL
             SELECT account, 0, debit, credit, 1 FROM voucher_lines
             WHERE company_id = ? AND fiscal_year_start = ?
         ) t
         LEFT JOIN accounts a ON a.company_id = ? AND a.number = t.account
         GROUP BY t.account
         HAVING MAX(t.in_year) = 1 OR SUM(t.opening) != 0
         ORDER BY t.account",
    )
    .bind(&company_id)
    .bind(has_opening)
    .bind(&company_id)
    .bind(&start)
    .bind(&company_id)
    .bind(&start)
    .bind(&company_id)
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(account, name, opening, debit, credit)| TrialBalanceRow {
            account,
            name,
            opening,
            debit,
            credit,
        })
        .collect())
}

/// One account's huvudbok for one fiscal year: its ingående balans, then its
/// lines by date, then voucher number, then line, each with the balance
/// after it.
// ponytail: whole year in one response; paginate when a year gets large.
pub async fn account_ledger(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
    fiscal_year_start: Date,
    account: u32,
) -> Result<AccountLedger> {
    let company = doris_company::get_company(pool, company_id, user_id).await?;
    let account = AccountNumber::parse(account)?;
    let has_opening = fiscal_year_start >= company.first_fiscal_year.start;
    let (company_id, start) = (company_id.to_string(), fiscal_year_start.to_string());
    let number = i64::from(account.get());
    // One read transaction: the opening balance and the lines from the same
    // snapshot (WAL).
    let mut tx = pool.begin().await?;
    let opening: i64 = if account.get() < 3000 {
        sqlx::query_scalar(
            "SELECT COALESCE(SUM(debit - credit), 0) FROM (
                 SELECT debit, credit FROM opening_balances
                 WHERE company_id = ? AND account = ? AND ?
                 UNION ALL
                 SELECT debit, credit FROM voucher_lines
                 WHERE company_id = ? AND fiscal_year_start < ? AND account = ?
             )",
        )
        .bind(&company_id)
        .bind(number)
        .bind(has_opening)
        .bind(&company_id)
        .bind(&start)
        .bind(number)
        .fetch_one(&mut *tx)
        .await?
    } else {
        0
    };
    let lines: Vec<(String, u32, String, i64, i64)> = sqlx::query_as(
        "SELECT v.date, v.number, v.text, l.debit, l.credit
         FROM voucher_lines l
         JOIN vouchers v ON v.company_id = l.company_id
             AND v.fiscal_year_start = l.fiscal_year_start AND v.number = l.number
         WHERE l.company_id = ? AND l.fiscal_year_start = ? AND l.account = ?
         ORDER BY v.date, v.number, l.line_no",
    )
    .bind(&company_id)
    .bind(&start)
    .bind(number)
    .fetch_all(&mut *tx)
    .await?;
    tx.commit().await?;
    let entries = running_balance(
        opening,
        lines
            .into_iter()
            .map(|(date, number, text, debit, credit)| {
                let date = date.parse().expect("projected dates are valid");
                (date, number, text, debit, credit)
            })
            .collect(),
    )
    .ok_or(Error::Overflow)?;
    Ok(AccountLedger { opening, entries })
}

/// The first fiscal year's ingående balanser, by account.
pub async fn opening_balances(
    pool: &SqlitePool,
    company_id: Uuid,
    user_id: Uuid,
) -> Result<Vec<VoucherLine>> {
    doris_company::get_company(pool, company_id, user_id).await?;
    let rows: Vec<(u32, i64, i64)> = sqlx::query_as(
        "SELECT account, debit, credit FROM opening_balances WHERE company_id = ? ORDER BY account",
    )
    .bind(company_id.to_string())
    .fetch_all(pool)
    .await?;
    Ok(rows
        .into_iter()
        .map(|(account, debit, credit)| {
            VoucherLine::new(account, debit, credit).expect("projected accounts are valid")
        })
        .collect())
}

/// Voucher `number` in `vouchers`. Numbers run 1..=n (the trigger
/// guarantees it), so number - 1 is the index. A row whose head is missing
/// (committed after the heads were read) gives `None`, never a panic.
fn voucher_at(vouchers: &mut [Voucher], number: u32) -> Option<&mut Voucher> {
    (number as usize)
        .checked_sub(1)
        .and_then(|i| vouchers.get_mut(i))
        .filter(|v| v.number == number)
}

/// Puts each `(number, account, debit, credit)` line on its voucher.
fn attach_lines(vouchers: &mut [Voucher], lines: Vec<(u32, u32, i64, i64)>) {
    for (number, account, debit, credit) in lines {
        if let Some(voucher) = voucher_at(vouchers, number) {
            voucher.lines.push(
                VoucherLine::new(account, debit, credit).expect("projected accounts are valid"),
            );
        }
    }
}

/// An underlag as the projection holds it.
fn projected_attachment(
    sha256: String,
    file_name: &str,
    content_type: &str,
    size: i64,
) -> Attachment {
    Attachment {
        sha256,
        file_name: AttachmentName::parse(file_name).expect("projected names are valid"),
        content_type: ContentType::from_mime(content_type).expect("projected types are valid"),
        size: size as u64,
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
            attachments: Vec::new(),
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
