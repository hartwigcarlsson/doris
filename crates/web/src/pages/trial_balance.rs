//! Saldobalans: every account with lines in one fiscal year, split into
//! balansräkning and resultaträkning.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{
    FiscalYearSelect, keep_year_in_url, opening_balances_missing, use_fiscal_years,
};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::use_query_map;

/// One part of the saldobalans and its totals.
#[derive(Debug, Default, PartialEq)]
pub struct Part {
    pub rows: Vec<lpb::TrialBalanceRow>,
    pub debit: i64,
    pub credit: i64,
}

impl Part {
    pub fn balance(&self) -> i64 {
        self.debit - self.credit
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
        part.debit += row.debit;
        part.credit += row.credit;
        part.rows.push(row);
    }
    (balance, income)
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
    let missing = move || years.with(|ys| opening_balances_missing(ys, &year.get()));

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Saldobalans"</h1>
            <ErrorAlert message=error />
            <FiscalYearSelect years=years year=year />
            <Show when=missing>
                <p class="text-xs/relaxed text-muted-foreground">
                    "Ingående balanser saknas än, så saldon för balansräkningens konton visar bara årets rörelser."
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
                    let total = balance.balance() + income.balance();
                    let result = -income.balance();
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
    let (debit, credit, balance) = (part.debit, part.credit, part.balance());
    view! {
        <section class="grid gap-2">
            <h2 class="text-sm font-medium">{title}</h2>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Saldo"</th>
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
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(row.debit - row.credit)}</td>
                                </tr>
                            }
                        })
                        .collect_view()}
                    <tr class=TABLE_ROW>
                        <td class=format!("{TABLE_CELL} font-medium") colspan="2">"Summa"</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                        <td class=TABLE_AMOUNT_CELL>{amount(balance)}</td>
                    </tr>
                </tbody>
            </Table>
        </section>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn row(account: u32, debit: i64, credit: i64) -> lpb::TrialBalanceRow {
        lpb::TrialBalanceRow {
            account,
            name: String::new(),
            debit,
            credit,
        }
    }

    #[test]
    fn accounts_below_3000_belong_to_the_balance_sheet() {
        let (balance, income) = split(vec![
            row(1930, 1000, 300),
            row(2999, 0, 50),
            row(3000, 0, 900),
            row(5010, 250, 0),
        ]);

        assert_eq!(
            balance.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [1930, 2999]
        );
        assert_eq!(
            (balance.debit, balance.credit, balance.balance()),
            (1000, 350, 650)
        );
        assert_eq!(
            income.rows.iter().map(|r| r.account).collect::<Vec<_>>(),
            [3000, 5010]
        );
        assert_eq!(
            (income.debit, income.credit, income.balance()),
            (250, 900, -650)
        );
    }

    #[test]
    fn no_rows_give_two_empty_parts() {
        assert_eq!(split(Vec::new()), (Part::default(), Part::default()));
    }
}
