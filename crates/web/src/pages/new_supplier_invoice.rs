//! Register a supplier invoice in the active company. The server books it
//! and decides its number.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::attachments::check_sizes;
use crate::errors::{describe, describe_code};
use crate::format::{amount, parse_amount, plus_days, today};
use crate::invoice_ui::{InvoiceLineRows, LineRow, PickedFiles, preview_vat};
use crate::task::spawn_local;
use crate::ui::{Button, ErrorAlert, Field, PageHeader, Panel, SELECT_OPTION, Select};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos_router::hooks::use_navigate;

#[component]
pub fn NewSupplierInvoice() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let navigate = use_navigate();
    let suppliers = RwSignal::new(Vec::<ipb::Supplier>::new());
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let supplier = RwSignal::new(String::new());
    let invoice_number = RwSignal::new(String::new());
    let invoice_date = RwSignal::new(today());
    let due_date = RwSignal::new(plus_days(&today(), 30).unwrap_or_default());
    let reference = RwSignal::new(String::new());
    let next_id = StoredValue::new(1u32);
    let rows = RwSignal::new(vec![LineRow::new(0)]);
    // The VAT field, and the computed amount it last showed: while they are
    // the same the user hasn't changed it, and it follows the lines.
    let vat = RwSignal::new(amount(0));
    let auto_vat = RwSignal::new(amount(0));
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
    let reading = RwSignal::new(0u32);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let form_company = StoredValue::new(String::new());

    Effect::new(move |_| {
        let date = invoice_date.get();
        if let Some(due) = plus_days(&date, 30) {
            due_date.set(due);
        }
    });
    Effect::new(move |_| {
        let lines: Vec<(i64, u32)> = rows.get().iter().map(LineRow::preview).collect();
        let computed = amount(preview_vat(&lines).iter().map(|(_, vat)| vat).sum());
        if vat.get_untracked() == auto_vat.get_untracked() {
            vat.set(computed.clone());
        }
        auto_vat.set(computed);
    });
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Nothing typed for the previous company may be registered in this one.
        suppliers.set(Vec::new());
        supplier.set(String::new());
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
            let listed = invoicing_api()
                .list_suppliers(ipb::ListSuppliersRequest {
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
                    .suppliers
                    .into_iter()
                    .filter(|s| s.active)
                    .collect();
                supplier.set(
                    active
                        .first()
                        .map(|s| s.number.to_string())
                        .unwrap_or_default(),
                );
                suppliers.set(active);
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
        let typed_vat = vat.get_untracked();
        let vat_override = if typed_vat == auto_vat.get_untracked() {
            None
        } else {
            match parse_amount(&typed_vat) {
                Some(ore) => Some(ore),
                None => return error.set(Some("Skriv beloppen som 1 234,50.".into())),
            }
        };
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
        busy.set(true);
        let company_id = form_company.get_value();
        let navigate = navigate.clone();
        spawn_local(async move {
            let request = ipb::RegisterSupplierInvoiceRequest {
                company_id: company_id.clone(),
                supplier_number: supplier.get_untracked().parse().unwrap_or(0),
                invoice_number: invoice_number.get_untracked(),
                invoice_date: invoice_date.get_untracked(),
                due_date: due_date.get_untracked(),
                reference: reference.get_untracked(),
                lines,
                vat: vat_override,
                attachments: files.get_untracked(),
            };
            let result = invoicing_api().register_supplier_invoice(request).await;
            busy.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(_) => navigate("/supplier-invoices", Default::default()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let net_total = move || rows.get().iter().map(|r| r.preview().0).sum::<i64>();
    let to_pay = move || net_total() + parse_amount(&vat.get()).unwrap_or(0);

    view! {
        <div class="grid gap-6">
        <PageHeader title="Ny leverantörsfaktura" />
        <Panel>
            <form class="grid gap-4" novalidate on:submit=submit>
                <ErrorAlert message=error />
                <div class="grid grid-cols-2 gap-4">
                    <Select label="Leverantör" id="invoice_supplier" value=supplier>
                        {move || {
                            suppliers
                                .get()
                                .into_iter()
                                .map(|s| {
                                    let name = s.details.map(|d| d.name).unwrap_or_default();
                                    view! {
                                        <option class=SELECT_OPTION value=s.number.to_string()>
                                            {format!("{} {}", s.number, name)}
                                        </option>
                                    }
                                })
                                .collect_view()
                        }}
                    </Select>
                    <Field label="Fakturanummer" id="invoice_number" value=invoice_number />
                    <Field label="Fakturadatum" id="invoice_date" kind="date" value=invoice_date />
                    <Field label="Förfallodatum" id="invoice_due_date" kind="date" value=due_date />
                    <Field label="OCR/meddelande" id="invoice_reference" value=reference />
                </div>
                <datalist id="invoice_accounts">
                    {move || {
                        accounts
                            .get()
                            .into_iter()
                            .filter(|a| a.active)
                            .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                            .collect_view()
                    }}
                </datalist>
                <InvoiceLineRows rows=rows next_id=next_id list="invoice_accounts" />
                <div class="grid grid-cols-[10rem_1fr] items-end gap-4">
                    <Field label="Moms" id="invoice_vat" value=vat />
                    <p class="text-xs/relaxed">
                        {move || format!("Netto {} · Att betala {}", amount(net_total()), amount(to_pay()))}
                    </p>
                </div>
                <PickedFiles id="invoice_files" files=files reading=reading error=error company=form_company />
                // In its own row, so the grid does not stretch it across the card.
                <div>
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>"Registrera"</Button>
                </div>
            </form>
        </Panel>
        </div>
    }
}
