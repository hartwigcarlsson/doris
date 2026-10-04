//! Räkenskapsår: the active company's fiscal years, closed and reopened
//! here. The server checks every rule; the buttons only show where they can
//! work.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::errors::describe;
use crate::fiscal_year::{closable, reopenable, use_fiscal_years};
use crate::format::today;
use crate::ui::{
    Button, ErrorAlert, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table,
    TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn FiscalYears() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let error = RwSignal::new(None::<String>);
    let done = RwSignal::new(None::<String>);
    let (years, _) = use_fiscal_years(String::new(), error);
    // Never say "closed" about the previous company.
    Effect::new(move |_| {
        companies.active.track();
        done.set(None);
    });
    // Kontantmetoden: unpaid customer and supplier invoices belong in the
    // year-end books (BFL 5 kap. 2 §), which Doris doesn't book yet.
    let unpaid_under_cash = RwSignal::new(false);
    Effect::new(move |_| {
        let company_id = companies.active.get();
        unpaid_under_cash.set(false);
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = invoicing_api();
            let suppliers = api
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let customers = api
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            let supplier_unpaid = suppliers.map(|r| {
                let r = r.into_inner();
                r.cash_method && r.invoices.iter().any(|i| i.status == "unpaid")
            });
            let customer_unpaid = customers.map(|r| {
                let r = r.into_inner();
                r.cash_method && r.invoices.iter().any(|i| i.status == "unpaid")
            });
            unpaid_under_cash
                .set(supplier_unpaid.unwrap_or(false) || customer_unpaid.unwrap_or(false));
        });
    });

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Räkenskapsår"</h1>
            <ErrorAlert message=error />
            {move || unpaid_under_cash.get().then(|| view! {
                <p class="text-xs/relaxed text-muted-foreground">
                    "Det finns obetalda kund- eller leverantörsfakturor. Med kontantmetoden ska de bokföras vid räkenskapsårets slut (BFL 5 kap. 2 §). Doris gör inte det än."
                </p>
            })}
            {move || done.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Räkenskapsår"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || years.get() key=|y| (y.start.clone(), y.closed) let(fiscal_year)>
                        <FiscalYearRow fiscal_year=fiscal_year years=years error=error done=done />
                    </For>
                </tbody>
            </Table>
            <A href="/opening-balances" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ingående balanser"</A>
        </div>
    }
}

#[component]
fn FiscalYearRow(
    fiscal_year: lpb::FiscalYear,
    years: RwSignal<Vec<lpb::FiscalYear>>,
    error: RwSignal<Option<String>>,
    done: RwSignal<Option<String>>,
) -> impl IntoView {
    let companies = expect_context::<Companies>();
    let start = StoredValue::new(fiscal_year.start.clone());
    let confirming = RwSignal::new(false);
    let reopening = RwSignal::new(false);
    let reason = RwSignal::new(String::new());
    let busy = RwSignal::new(false);
    let can_close = move || years.with(|ys| closable(ys, &today())) == Some(start.get_value());
    let can_reopen = move || years.with(|ys| reopenable(ys)) == Some(start.get_value());
    // Once the server has answered, the list says so; the row is rebuilt.
    let mark = move |start: &str, closed: bool| {
        years.update(|ys| {
            for y in ys.iter_mut().filter(|y| y.start == start) {
                y.closed = closed;
            }
        })
    };

    let close = move |_| {
        error.set(None);
        done.set(None);
        busy.set(true);
        let company_id = companies.active.get_untracked();
        let start = start.get_value();
        spawn_local(async move {
            let result = ledger_api()
                .close_fiscal_year(lpb::CloseFiscalYearRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // The row may have been rebuilt meanwhile; never read it after the await.
            busy.try_set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    done.set(Some(match response.into_inner().result_voucher {
                        0 => "Räkenskapsåret stängt.".to_owned(),
                        n => format!("Räkenskapsåret stängt. Resultatet bokfördes som ver {n}."),
                    }));
                    mark(&start, true);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let reopen = move |_| {
        error.set(None);
        done.set(None);
        busy.set(true);
        let company_id = companies.active.get_untracked();
        let start = start.get_value();
        let reason = reason.get_untracked();
        spawn_local(async move {
            let result = ledger_api()
                .reopen_fiscal_year(lpb::ReopenFiscalYearRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                    reason,
                })
                .await;
            // The row may have been rebuilt meanwhile; never read it after the await.
            busy.try_set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => {
                    done.set(Some("Räkenskapsåret öppnat igen.".to_owned()));
                    mark(&start, false);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{format!("{} – {}", fiscal_year.start, fiscal_year.end)}</td>
            <td class=TABLE_CELL>{if fiscal_year.closed { "Stängt" } else { "Öppet" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || can_close() && !confirming.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>"Stäng år"</Button>
                </Show>
                <Show when=move || confirming.get()>
                    <span class="inline-flex items-center gap-2 whitespace-normal">
                        <span class="text-xs/relaxed text-muted-foreground">"Årets resultat bokförs som en verifikation och året låses för bokföring."</span>
                        <Button kind="button" disabled=busy on:click=close>"Bekräfta stängning"</Button>
                    </span>
                </Show>
                <Show when=move || can_reopen() && !reopening.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| reopening.set(true)>"Öppna igen"</Button>
                </Show>
                <Show when=move || reopening.get()>
                    <span class="inline-flex items-center gap-2">
                        <TextInput label="Anledning" value=reason />
                        <Button kind="button" disabled=busy on:click=reopen>"Bekräfta"</Button>
                    </span>
                </Show>
            </td>
        </tr>
    }
}
