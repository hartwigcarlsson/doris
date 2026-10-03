//! Huvudbok for one account: its lines in one fiscal year, by date, with
//! the running balance.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url, use_fiscal_years};
use crate::format::amount;
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_params_map, use_query_map};

#[component]
pub fn AccountLedger() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // Junk in the URL becomes 0, which the server refuses as
    // invalid_account_number, shown in Swedish.
    let account = use_params_map()
        .read_untracked()
        .get("account")
        .and_then(|a| a.parse::<u32>().ok())
        .unwrap_or(0);
    let error = RwSignal::new(None::<String>);
    let preferred = use_query_map()
        .read_untracked()
        .get("fy")
        .unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    if account != 0 {
        keep_year_in_url(format!("/trial-balance/{account}"), year);
    }
    let name = RwSignal::new(String::new());
    // The opening balance and entries; None until the chosen year's arrive.
    let entries = RwSignal::new(None::<(i64, Vec<lpb::LedgerEntry>)>);

    Effect::new(move |_| {
        let company_id = companies.active.get();
        name.set(String::new());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = result {
                let found = response
                    .into_inner()
                    .accounts
                    .into_iter()
                    .find(|a| a.number == account);
                name.set(found.map(|a| a.name).unwrap_or_default());
            }
        });
    });
    Effect::new(move |_| {
        let start = year.get();
        let company_id = companies.active.get_untracked();
        entries.set(None);
        error.set(None);
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .get_account_ledger(lpb::GetAccountLedgerRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                    account,
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    let r = response.into_inner();
                    entries.set(Some((r.opening, r.entries)))
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">{move || format!("{account} {}", name.get()).trim_end().to_owned()}</h1>
                <A href=move || format!("/trial-balance?fy={}", year.get()) attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">
                    "Tillbaka till saldobalansen"
                </A>
            </div>
            <ErrorAlert message=error />
            <FiscalYearSelect years=years year=year />
            {move || {
                entries.get().map(|(opening, entries)| {
                    if entries.is_empty() && opening == 0 {
                        return view! {
                            <p class="text-xs/relaxed text-muted-foreground">"Inga transaktioner på kontot under räkenskapsåret."</p>
                        }
                        .into_any();
                    }
                    let debit: i64 = entries.iter().map(|e| e.debit).sum();
                    let credit: i64 = entries.iter().map(|e| e.credit).sum();
                    let balance = entries.last().map_or(opening, |e| e.balance);
                    view! {
                        <Table>
                            <thead class=TABLE_HEAD>
                                <tr class=TABLE_ROW>
                                    <th class=TABLE_HEADER_CELL>"Datum"</th>
                                    <th class=TABLE_HEADER_CELL>"Ver"</th>
                                    <th class=TABLE_HEADER_CELL>"Text"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Saldo"</th>
                                </tr>
                            </thead>
                            <tbody class=TABLE_BODY>
                                {(opening != 0).then(|| view! {
                                    <tr class=TABLE_ROW>
                                        <td class=TABLE_CELL></td>
                                        <td class=TABLE_CELL></td>
                                        <td class=TABLE_CELL>"Ingående balans"</td>
                                        <td class=TABLE_AMOUNT_CELL></td>
                                        <td class=TABLE_AMOUNT_CELL></td>
                                        <td class=TABLE_AMOUNT_CELL>{amount(opening)}</td>
                                    </tr>
                                })}
                                {entries
                                    .into_iter()
                                    .map(|e| {
                                        view! {
                                            <tr class=TABLE_ROW>
                                                <td class=TABLE_CELL>{e.date}</td>
                                                <td class=TABLE_CELL>{e.number}</td>
                                                <td class=TABLE_CELL>{e.text}</td>
                                                <td class=TABLE_AMOUNT_CELL>{(e.debit != 0).then(|| amount(e.debit))}</td>
                                                <td class=TABLE_AMOUNT_CELL>{(e.credit != 0).then(|| amount(e.credit))}</td>
                                                <td class=TABLE_AMOUNT_CELL>{amount(e.balance)}</td>
                                            </tr>
                                        }
                                    })
                                    .collect_view()}
                                <tr class=TABLE_ROW>
                                    <td class=format!("{TABLE_CELL} font-medium") colspan="3">"Summa"</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(debit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(credit)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(balance)}</td>
                                </tr>
                            </tbody>
                        </Table>
                    }
                    .into_any()
                })
            }}
        </div>
    }
}
