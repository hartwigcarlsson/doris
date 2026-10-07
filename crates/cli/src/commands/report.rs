use super::Context;
use crate::amount::{display, kronor};
use crate::output::{Failure, Output};
use clap::Subcommand;
use doris_proto::ledger::v1 as lpb;
use serde_json::json;

#[derive(Subcommand)]
pub enum ReportAction {
    /// Saldobalans för räkenskapsåret.
    TrialBalance {
        /// Räkenskapsår: ett år eller ett startdatum. Standard är året som innehåller idag.
        #[arg(long)]
        year: Option<String>,
    },
    /// Huvudbok för ett konto.
    Ledger {
        account: u32,
        #[arg(long)]
        year: Option<String>,
    },
    /// Resultat- och balansräkning.
    Statements {
        #[arg(long)]
        year: Option<String>,
    },
}

pub async fn trial_balance(
    context: &Context,
    output: &mut Output<'_>,
    year: Option<&str>,
) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let start = super::fiscal_year(context, &company.id, year).await?;
    let rows = context
        .doris
        .ledger()
        .get_trial_balance(context.doris.request(lpb::GetTrialBalanceRequest {
            company_id: company.id,
            fiscal_year_start: start.clone(),
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner()
        .rows;
    let closing = |r: &lpb::TrialBalanceRow| r.opening + r.debit - r.credit;
    let value: Vec<_> = rows
        .iter()
        .map(|r| {
            json!({
                "account": r.account, "name": r.name, "opening": kronor(r.opening),
                "debit": kronor(r.debit), "credit": kronor(r.credit),
                "closing": kronor(closing(r)),
            })
        })
        .collect();
    let mut text = format!(
        "Konto  {:<30}  {:>12}  {:>12}  {:>12}  {:>12}\n",
        "Namn", "IB", "Debet", "Kredit", "UB"
    );
    for r in &rows {
        text.push_str(&format!(
            "{:<5}  {:<30}  {:>12}  {:>12}  {:>12}  {:>12}\n",
            r.account,
            r.name,
            display(r.opening),
            display(r.debit),
            display(r.credit),
            display(closing(r))
        ));
    }
    context.print(
        output,
        json!({"fiscal_year_start": start, "rows": value}),
        text,
    );
    Ok(())
}

pub async fn ledger(
    context: &Context,
    output: &mut Output<'_>,
    account: u32,
    year: Option<&str>,
) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let start = super::fiscal_year(context, &company.id, year).await?;
    let response = context
        .doris
        .ledger()
        .get_account_ledger(context.doris.request(lpb::GetAccountLedgerRequest {
            company_id: company.id,
            fiscal_year_start: start.clone(),
            account,
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    let entries: Vec<_> = response
        .entries
        .iter()
        .map(|e| {
            json!({
                "date": e.date, "number": e.number, "text": e.text,
                "debit": kronor(e.debit), "credit": kronor(e.credit),
                "balance": kronor(e.balance),
            })
        })
        .collect();
    let mut text = format!("Ingående balans {}\n", display(response.opening));
    for e in &response.entries {
        text.push_str(&format!(
            "{}  {:>4}  {:>12}  {:>12}  {:>12}  {}\n",
            e.date,
            e.number,
            display(e.debit),
            display(e.credit),
            display(e.balance),
            e.text
        ));
    }
    context.print(
        output,
        json!({
            "fiscal_year_start": start, "account": account,
            "opening": kronor(response.opening), "entries": entries,
        }),
        text,
    );
    Ok(())
}

fn kind(line: &lpb::StatementLine) -> String {
    lpb::StatementLineKind::try_from(line.kind)
        .map(|k| k.as_str_name())
        .unwrap_or("STATEMENT_LINE_KIND_UNSPECIFIED")
        .trim_start_matches("STATEMENT_LINE_KIND_")
        .to_lowercase()
}

fn lines_json(lines: &[lpb::StatementLine]) -> Vec<serde_json::Value> {
    lines
        .iter()
        .map(|l| {
            let heading = kind(l) == "heading";
            json!({
                "label": l.label, "kind": kind(l),
                "amount": (!heading).then(|| kronor(l.amount)),
                "previous": l.previous.map(kronor),
            })
        })
        .collect()
}

fn lines_text(title: &str, lines: &[lpb::StatementLine]) -> String {
    let mut text = format!("{title}\n");
    for l in lines {
        match kind(l).as_str() {
            "heading" => text.push_str(&format!("{}\n", l.label)),
            _ => {
                let previous = l.previous.map(display).unwrap_or_default();
                text.push_str(&format!(
                    "  {:<40}  {:>14}  {:>14}\n",
                    l.label,
                    display(l.amount),
                    previous
                ));
            }
        }
    }
    text
}

pub async fn statements(
    context: &Context,
    output: &mut Output<'_>,
    year: Option<&str>,
) -> Result<(), Failure> {
    let company = super::company(context).await?;
    let start = super::fiscal_year(context, &company.id, year).await?;
    let r = context
        .doris
        .ledger()
        .get_financial_statements(context.doris.request(lpb::GetFinancialStatementsRequest {
            company_id: company.id,
            fiscal_year_start: start.clone(),
        }))
        .await
        .map_err(|s| Failure::from_status(&s))?
        .into_inner();
    let previous =
        (!r.previous_fiscal_year_start.is_empty()).then_some(&r.previous_fiscal_year_start);
    let text = format!(
        "{}\n{}",
        lines_text("Resultaträkning", &r.income_statement),
        lines_text("Balansräkning", &r.balance_sheet)
    );
    context.print(
        output,
        json!({
            "fiscal_year_start": start, "previous_fiscal_year_start": previous,
            "income_statement": lines_json(&r.income_statement),
            "balance_sheet": lines_json(&r.balance_sheet),
            "difference": kronor(r.difference),
        }),
        text,
    );
    Ok(())
}
