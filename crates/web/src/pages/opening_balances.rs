//! Ingående balanser for the active company's first fiscal year, typed in
//! by a company that moves to Doris with history. Later years' are derived
//! by the server. Read-only once the first year is closed.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::use_fiscal_years;
use crate::format::amount;
use crate::ui::{
    Button, ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use crate::voucher_lines::{LineRows, Lines};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn OpeningBalances() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let error = RwSignal::new(None::<String>);
    let saved = RwSignal::new(None::<String>);
    let (years, _) = use_fiscal_years(String::new(), error);
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    // What the server holds; None until it has answered.
    let current = RwSignal::new(None::<Vec<lpb::VoucherLine>>);
    let lines = Lines::new();
    let busy = RwSignal::new(false);
    // The company this form was filled for; a save only ever goes there.
    let form_company = StoredValue::new(String::new());
    // Years are newest first, so the first year is the last one.
    let first = move || years.with(|ys| ys.last().cloned());

    Effect::new(move |_| {
        let company_id = companies.active.get();
        accounts.set(Vec::new());
        current.set(None);
        saved.set(None);
        lines.clear();
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = ledger_api();
            let balances = api
                .get_opening_balances(lpb::GetOpeningBalancesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let chart = api
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = chart {
                accounts.set(response.into_inner().accounts);
            }
            match balances {
                Ok(response) => {
                    let list = response.into_inner().lines;
                    lines.fill(&list);
                    current.set(Some(list));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        saved.set(None);
        let Some(request_lines) = lines.request() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        busy.set(true);
        let company_id = form_company.get_value();
        spawn_local(async move {
            let result = ledger_api()
                .set_opening_balances(lpb::SetOpeningBalancesRequest {
                    company_id: company_id.clone(),
                    lines: request_lines.clone(),
                })
                .await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => {
                    saved.set(Some("Ingående balanser sparade".into()));
                    current.set(Some(request_lines));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">
                {move || first().map(|y| format!("Ingående balanser {}", y.start)).unwrap_or_else(|| "Ingående balanser".into())}
            </h1>
            <ErrorAlert message=error />
            {move || saved.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Show
                when=move || first().is_some_and(|y| y.closed)
                fallback=move || view! {
                    <form class="grid gap-4" novalidate on:submit=submit>
                        <datalist id="balance_accounts">
                            {move || {
                                accounts
                                    .get()
                                    .into_iter()
                                    .filter(|a| a.number < 3000)
                                    .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                                    .collect_view()
                            }}
                        </datalist>
                        <LineRows lines=lines list="balance_accounts" />
                        <div>
                            <Button disabled=Signal::derive(move || busy.get() || current.with(Option::is_none))>"Spara"</Button>
                        </div>
                    </form>
                }
            >
                <p class="text-xs/relaxed text-muted-foreground">
                    "Räkenskapsåret är stängt, så de ingående balanserna kan inte ändras."
                </p>
                <Table>
                    <thead class=TABLE_HEAD>
                        <tr class=TABLE_ROW>
                            <th class=TABLE_HEADER_CELL>"Konto"</th>
                            <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                            <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                        </tr>
                    </thead>
                    <tbody class=TABLE_BODY>
                        {move || {
                            current
                                .get()
                                .unwrap_or_default()
                                .into_iter()
                                .map(|l| view! {
                                    <tr class=TABLE_ROW>
                                        <td class=TABLE_CELL>{l.account}</td>
                                        <td class=TABLE_AMOUNT_CELL>{(l.debit != 0).then(|| amount(l.debit))}</td>
                                        <td class=TABLE_AMOUNT_CELL>{(l.credit != 0).then(|| amount(l.credit))}</td>
                                    </tr>
                                })
                                .collect_view()
                        }}
                    </tbody>
                </Table>
            </Show>
        </div>
    }
}
