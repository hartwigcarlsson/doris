//! Register a customer invoice in the active company. The number is
//! proposed by the server and may be changed; the server books it.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::attachments::check_sizes;
use crate::errors::{describe, describe_code};
use crate::format::{amount, plus_days, today};
use crate::invoice_ui::{InvoiceLineRows, LineRow, PickedFiles, preview_vat};
use crate::ui::{Button, Card, ErrorAlert, Field, SELECT_OPTION, Select};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

#[component]
pub fn NewCustomerInvoice() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let customers = RwSignal::new(Vec::<ipb::Customer>::new());
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let customer = RwSignal::new(String::new());
    let invoice_number = RwSignal::new(String::new());
    let invoice_date = RwSignal::new(today());
    let due_date = RwSignal::new(today());
    let reference = RwSignal::new(String::new());
    let next_id = StoredValue::new(1u32);
    let rows = RwSignal::new(vec![LineRow::new(0)]);
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
    let reading = RwSignal::new(0u32);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let form_company = StoredValue::new(String::new());

    // The due date follows the invoice date and the chosen customer's terms.
    Effect::new(move |_| {
        let date = invoice_date.get();
        let chosen = customer.get();
        let terms = customers.with(|all| {
            all.iter()
                .find(|c| c.number.to_string() == chosen)
                .and_then(|c| c.details.as_ref())
                .map_or(30, |d| d.payment_terms)
        });
        if let Some(due) = plus_days(&date, i64::from(terms)) {
            due_date.set(due);
        }
    });
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Nothing typed for the previous company may be registered in this one.
        customers.set(Vec::new());
        customer.set(String::new());
        accounts.set(Vec::new());
        invoice_number.set(String::new());
        reference.set(String::new());
        rows.set(vec![LineRow::new(next_id.get_value())]);
        next_id.update_value(|id| *id += 1);
        files.set(Vec::new());
        error.set(None);
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = invoicing_api();
            let listed = api
                .list_customers(ipb::ListCustomersRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let invoices = api
                .list_customer_invoices(ipb::ListCustomerInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let chart = ledger_api()
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = listed {
                let active: Vec<_> = response
                    .into_inner()
                    .customers
                    .into_iter()
                    .filter(|c| c.active)
                    .collect();
                customer.set(
                    active
                        .first()
                        .map(|c| c.number.to_string())
                        .unwrap_or_default(),
                );
                customers.set(active);
            }
            if let Ok(response) = invoices {
                invoice_number.set(response.into_inner().next_invoice_number);
            }
            if let Ok(response) = chart {
                accounts.set(response.into_inner().accounts);
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        if reading.get_untracked() > 0 {
            return;
        }
        let Some(lines) = rows
            .get_untracked()
            .iter()
            .map(LineRow::request)
            .collect::<Option<Vec<_>>>()
        else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
        busy.set(true);
        let company_id = form_company.get_value();
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = ipb::RegisterCustomerInvoiceRequest {
                company_id: company_id.clone(),
                customer_number: customer.get_untracked().parse().unwrap_or(0),
                invoice_number: invoice_number.get_untracked(),
                invoice_date: invoice_date.get_untracked(),
                due_date: due_date.get_untracked(),
                reference: reference.get_untracked(),
                lines,
                attachments: files.get_untracked(),
            };
            let result = invoicing_api().register_customer_invoice(request).await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => navigate("/customer-invoices", Default::default()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let lines = move || rows.get().iter().map(LineRow::preview).collect::<Vec<_>>();
    let summary = move || {
        let lines = lines();
        let net: i64 = lines.iter().map(|(net, _)| net).sum();
        let vat = preview_vat(&lines);
        let vat_text = vat
            .iter()
            .map(|(rate, ore)| format!("{rate} %: {}", amount(*ore)))
            .collect::<Vec<_>>()
            .join(", ");
        let total = net + vat.iter().map(|(_, ore)| ore).sum::<i64>();
        format!(
            "Netto {} · Moms {} · Att betala {}",
            amount(net),
            if vat_text.is_empty() {
                amount(0)
            } else {
                vat_text
            },
            amount(total)
        )
    };

    view! {
        <Card title="Ny kundfaktura">
            <form class="grid gap-4" data-wide novalidate on:submit=submit>
                <ErrorAlert message=error />
                <div class="grid grid-cols-2 gap-4">
                    <Select label="Kund" id="invoice_customer" value=customer>
                        {move || {
                            customers
                                .get()
                                .into_iter()
                                .map(|c| {
                                    let name = c.details.map(|d| d.name).unwrap_or_default();
                                    view! { <option class=SELECT_OPTION value=c.number.to_string()>{format!("{} {}", c.number, name)}</option> }
                                })
                                .collect_view()
                        }}
                    </Select>
                    <Field label="Fakturanummer" id="customer_invoice_number" value=invoice_number />
                    <Field label="Fakturadatum" id="customer_invoice_date" kind="date" value=invoice_date />
                    <Field label="Förfallodatum" id="customer_invoice_due_date" kind="date" value=due_date />
                    <Field label="OCR/meddelande" id="customer_invoice_reference" value=reference />
                </div>
                <datalist id="customer_invoice_accounts">
                    {move || {
                        accounts
                            .get()
                            .into_iter()
                            .filter(|a| a.active)
                            .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                            .collect_view()
                    }}
                </datalist>
                <InvoiceLineRows rows=rows next_id=next_id list="customer_invoice_accounts" />
                <p class="text-xs/relaxed">{summary}</p>
                <PickedFiles id="customer_invoice_files" files=files reading=reading error=error company=form_company />
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>"Registrera"</Button>
            </form>
        </Card>
    }
}
