//! Resultat- och balansräkning for one fiscal year, with the year before.
//! The server sends finished lines; this page only draws them.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, is_closed, keep_year_in_url, period, use_fiscal_years};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_query_map;

#[component]
pub fn FinancialStatements() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    keep_year_in_url("/financial-statements".into(), year);
    // None until the chosen year's statements have arrived.
    let statements = RwSignal::new(None::<lpb::GetFinancialStatementsResponse>);

    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        statements.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_financial_statements(lpb::GetFinancialStatementsRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => statements.set(Some(response.into_inner())),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let closed = move || years.with(|ys| is_closed(ys, &year.get()));

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Resultat- och balansräkning"</h1>
            <ErrorAlert message=error />
            <div class="flex items-end gap-4">
                <FiscalYearSelect years=years year=year />
                <Show when=closed>
                    <span class="pb-2 text-xs/relaxed text-muted-foreground">"Räkenskapsåret är stängt"</span>
                </Show>
            </div>
            {move || {
                statements.get().map(|s| {
                    let current = years.with_untracked(|ys| period(ys, &year.get_untracked()));
                    let previous = (!s.previous_fiscal_year_start.is_empty())
                        .then(|| years.with_untracked(|ys| period(ys, &s.previous_fiscal_year_start)));
                    let differences: Vec<i64> = std::iter::once(s.difference)
                        .chain(s.previous_difference)
                        .filter(|d| *d != 0)
                        .collect();
                    view! {
                        <StatementTable title="Resultaträkning" lines=s.income_statement current=current.clone() previous=previous.clone() />
                        <StatementTable title="Balansräkning" lines=s.balance_sheet current=current previous=previous />
                        {differences
                            .into_iter()
                            .map(|d| view! {
                                <p class="text-xs/relaxed text-destructive">
                                    {format!(
                                        "Balansräkningen balanserar inte (differens {}). Ett tidigare räkenskapsår är inte stängt, så dess resultat finns inte i eget kapital.",
                                        amount(d)
                                    )}
                                </p>
                            })
                            .collect_view()}
                    }
                })
            }}
        </div>
    }
}

/// One statement: headings in bold without amounts, items indented,
/// subtotals in bold under a rule.
#[component]
fn StatementTable(
    title: &'static str,
    lines: Vec<lpb::StatementLine>,
    current: String,
    previous: Option<String>,
) -> impl IntoView {
    let has_previous = previous.is_some();
    view! {
        <section class="grid gap-2">
            <h2 class="text-sm font-medium">{title}</h2>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Post"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>{current}</th>
                        {previous.map(|p| view! { <th class=format!("{TABLE_HEADER_CELL} text-right")>{p}</th> })}
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {lines
                        .into_iter()
                        .map(|line| {
                            let kind = line.kind();
                            let previous = line.previous;
                            match kind {
                                lpb::StatementLineKind::Heading => view! {
                                    <tr class=TABLE_ROW>
                                        <td class=format!("{TABLE_CELL} font-medium") colspan=if has_previous { "3" } else { "2" }>{line.label}</td>
                                    </tr>
                                }
                                .into_any(),
                                _ => {
                                    let subtotal = kind == lpb::StatementLineKind::Subtotal;
                                    view! {
                                        <tr class=if subtotal { format!("{TABLE_ROW} border-t font-medium") } else { TABLE_ROW.to_owned() }>
                                            <td class=if subtotal { TABLE_CELL.to_owned() } else { format!("{TABLE_CELL} pl-6") }>{line.label}</td>
                                            <td class=TABLE_AMOUNT_CELL>{amount(line.amount)}</td>
                                            {has_previous.then(|| view! { <td class=TABLE_AMOUNT_CELL>{amount(previous.unwrap_or(0))}</td> })}
                                        </tr>
                                    }
                                    .into_any()
                                }
                            }
                        })
                        .collect_view()}
                </tbody>
            </Table>
        </section>
    }
}
