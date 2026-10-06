//! Saldobalans: every account with lines in one fiscal year, split into
//! balansräkning and resultaträkning.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{
    FiscalYearSelect, is_closed, keep_year_in_url, opening_balances_preliminary, use_fiscal_years,
};
use crate::format::amount;
use crate::task::spawn_local;
use crate::ui::{
    Badge, ErrorAlert, PageHeader, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD,
    TABLE_HEADER_CELL, TABLE_ROW, Table, TableCard,
};
use leptos::prelude::*;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

/// One part of the saldobalans and its totals.
#[derive(Debug, Default, PartialEq)]
pub struct Part {
    pub rows: Vec<lpb::TrialBalanceRow>,
    pub opening: i64,
    pub debit: i64,
    pub credit: i64,
}

impl Part {
    /// The utgående balans.
    pub fn closing(&self) -> i64 {
        self.opening + self.debit - self.credit
    }
}

/// Balansräkning (accounts 1000–2999) and resultaträkning (3000–8999).
// ponytail: i64 totals; a year's lines would need ~92 biljarder kronor to overflow.
pub fn split(rows: Vec<lpb::TrialBalanceRow>) -> (Part, Part) {
    let (mut balance, mut income) = (Part::default(), Part::default());
    for row in rows {
        let part = if row.account < 3000 {
            &mut balance
        } else {
            &mut income
        };
        part.opening += row.opening;
        part.debit += row.debit;
        part.credit += row.credit;
        part.rows.push(row);
    }
    (balance, income)
}

/// The year's result as a profit is positive, leaving out 8999: once the
/// year is closed, the result voucher on 8999 would cancel it to 0.
pub fn computed_result(income: &Part) -> i64 {
    -income
        .rows
        .iter()
        .filter(|r| r.account != 8999)
        .map(|r| r.debit - r.credit)
        .sum::<i64>()
}

#[component]
pub fn TrialBalance() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    keep_year_in_url("/trial-balance".into(), year);
    // None until the chosen year's rows have arrived.
    let rows = RwSignal::new(None::<Vec<lpb::TrialBalanceRow>>);

    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        rows.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_trial_balance(lpb::GetTrialBalanceRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => rows.set(Some(response.into_inner().rows)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let preliminary = move || years.with(|ys| opening_balances_preliminary(ys, &year.get()));
    let closed = move || years.with(|ys| is_closed(ys, &year.get()));

    view! {
        <div class="grid gap-6">
            <PageHeader title="Saldobalans">
                <Show when=closed>
                    <Badge>"Stängt"</Badge>
                </Show>
                <FiscalYearSelect years=years year=year />
            </PageHeader>
            <ErrorAlert message=error />
            <Show when=preliminary>
                <p class="text-xs/relaxed text-muted-foreground">
                    "Föregående räkenskapsår är inte stängt, så de ingående balanserna är preliminära."
                </p>
            </Show>
            {move || {
                rows.get().map(|rows| {
                    if rows.is_empty() {
                        return view! {
                            <p class="text-xs/relaxed text-muted-foreground">"Inga verifikationer under räkenskapsåret."</p>
                        }
                        .into_any();
                    }
                    let start = year.get_untracked();
                    let (balance, income) = split(rows);
                    let total = balance.closing() + income.closing();
                    let result = computed_result(&income);
                    view! {
                        <PartTable title="Balansräkning" part=balance year=start.clone() />
                        <PartTable title="Resultaträkning" part=income year=start />
                        <div class="grid gap-1 text-xs/relaxed tabular-nums">
                            <p>"Beräknat resultat " {amount(result)}</p>
                            <p class=if total == 0 { "" } else { "text-destructive" }>"Summa saldo " {amount(total)}</p>
                        </div>
                    }
                    .into_any()
                })
            }}
        </div>
    }
}

/// One part of the saldobalans. Each account links to its huvudbok for `year`.
#[component]
fn PartTable(title: &'static str, part: Part, year: String) -> impl IntoView {
    let (opening, debit, credit, closing) = (part.opening, part.debit, part.credit, part.closing());
    view! {
        <TableCard>
            <h2 class="px-2 pt-1 text-sm font-medium">{title}</h2>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Ingående"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Utgående"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {part
                        .rows
                        .into_iter()
                        .map(|row| {
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>
                                        <A href=format!("/trial-balance/{}?fy={year}", row.account) attr:class="underline-offset-4 hover:underline">
                                            {row.account}
                                        </A>
                                    </td>
                                    <td class=TABLE_CELL>{row.name.clone()}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.opening)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.opening + row.debit - row.credit)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                    <tr class=TABLE_ROW>
                        <td class=format!("{TABLE_CELL} font-medium") colspan="2">"Summa"</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(opening)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(closing)}</td>
                    </tr>
                </tbody>
            </Table>
        </TableCard>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(account: u32, opening: i64, debit: i64, credit: i64) -> lpb::TrialBalanceRow {
        lpb::TrialBalanceRow {
            account,
            name: String::new(),
            opening,
            debit,
            credit,
        }
    }

    #[test]
    fn accounts_below_3000_belong_to_the_balance_sheet() {
        let (balance, income) = split(vec![
            row(1930, 500, 1000, 300),
            row(2999, -500, 0, 50),
            row(3000, 0, 0, 900),
            row(5010, 0, 250, 0),
        ]);

        assert_eq!(
            balance.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [1930, 2999]
        );
        assert_eq!(
            (
                balance.opening,
                balance.debit,
                balance.credit,
                balance.closing()
            ),
            (0, 1000, 350, 650)
        );
        assert_eq!(
            income.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [3000, 5010]
        );
        assert_eq!(
            (
                income.opening,
                income.debit,
                income.credit,
                income.closing()
            ),
            (0, 250, 900, -650)
        );
    }

    #[test]
    fn no_rows_give_two_empty_parts() {
        assert_eq!(split(Vec::new()), (Part::default(), Part::default()));
    }

    #[test]
    fn the_computed_result_leaves_out_8999_so_it_survives_closing() {
        let (_, income) = split(vec![
            row(3001, 0, 0, 1000),
            row(5010, 0, 300, 0),
            row(8999, 0, 700, 0),
        ]);
        assert_eq!(computed_result(&income), 700);
    }
}
