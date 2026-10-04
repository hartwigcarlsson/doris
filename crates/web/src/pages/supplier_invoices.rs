//! Leverantörsfakturor: the active company's supplier invoices. They are
//! paid, cancelled and payments reversed from here; the server books it all.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb};
use crate::attachments::{open_in, size_label};
use crate::errors::{describe, describe_code};
use crate::format::{amount, today};
use crate::ui::{
    Button, Checkbox, ErrorAlert, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW,
    Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// Obetald, Förfallen (unpaid past its due date), Betald or Makulerad.
fn status_label(invoice: &ipb::SupplierInvoice, today: &str) -> &'static str {
    match invoice.status.as_str() {
        "paid" => "Betald",
        "cancelled" => "Makulerad",
        _ if invoice.due_date.as_str() < today => "Förfallen",
        _ => "Obetald",
    }
}

#[component]
pub fn SupplierInvoices() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The invoices and the company they were loaded for, set together.
    let invoices = RwSignal::new((String::new(), Vec::<ipb::SupplierInvoice>::new()));
    let show_all = RwSignal::new(false);
    let error = RwSignal::new(None::<String>);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = invoicing_api()
                .list_supplier_invoices(ipb::ListSupplierInvoicesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => invoices.set((company_id, response.into_inner().invoices)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        invoices.set((String::new(), Vec::new()));
        error.set(None);
        load();
    });
    let changed = Callback::new(move |()| load());

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-center justify-between">
                <h1 class="text-sm font-medium">"Leverantörsfakturor"</h1>
                <A href="/supplier-invoices/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">
                    "Ny leverantörsfaktura"
                </A>
            </div>
            <ErrorAlert message=error />
            <Checkbox label="Visa betalda och makulerade" id="show_all_invoices" checked=show_all />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Leverantör"</th>
                        <th class=TABLE_HEADER_CELL>"Fakturanr"</th>
                        <th class=TABLE_HEADER_CELL>"Fakturadatum"</th>
                        <th class=TABLE_HEADER_CELL>"Förfaller"</th>
                        <th class=TABLE_HEADER_CELL>"Belopp"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = invoices.get();
                            list.into_iter()
                                .filter(|i| i.status == "unpaid" || show_all.get())
                                .map(|i| (company_id.clone(), i))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, i)| (company_id.clone(), i.number, i.status.clone(), i.vouchers.len())
                        let((company_id, invoice))
                    >
                        <InvoiceRow company_id=company_id invoice=invoice changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[derive(Clone, Copy, PartialEq)]
enum Panel {
    Closed,
    Details,
    Pay,
    Cancel,
    Reverse,
}

#[component]
fn InvoiceRow(
    company_id: String,
    invoice: ipb::SupplierInvoice,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let number = invoice.number;
    let label = status_label(&invoice, &today());
    let (unpaid, paid) = (invoice.status == "unpaid", invoice.status == "paid");
    let invoice = StoredValue::new(invoice);
    let panel = RwSignal::new(Panel::Closed);
    let pay_date = RwSignal::new(today());
    let pay_account = RwSignal::new("1930".to_string());
    let reason = RwSignal::new(String::new());
    let toggle = move |wanted: Panel| {
        error.set(None);
        panel.update(|p| *p = if *p == wanted { Panel::Closed } else { wanted });
    };

    let act = move |action: Panel| {
        error.set(None);
        spawn_local(async move {
            let company_id = company_id.get_value();
            let mut api = invoicing_api();
            let result = match action {
                Panel::Pay => api
                    .pay_supplier_invoice(ipb::PaySupplierInvoiceRequest {
                        company_id,
                        number,
                        date: pay_date.get_untracked(),
                        account: pay_account.get_untracked().trim().parse().unwrap_or(0),
                    })
                    .await
                    .map(|_| ()),
                Panel::Cancel => api
                    .cancel_supplier_invoice(ipb::CancelSupplierInvoiceRequest {
                        company_id,
                        number,
                        reason: reason.get_untracked(),
                    })
                    .await
                    .map(|_| ()),
                Panel::Reverse => api
                    .reverse_supplier_invoice_payment(ipb::ReverseSupplierInvoicePaymentRequest {
                        company_id,
                        number,
                        reason: reason.get_untracked(),
                    })
                    .await
                    .map(|_| ()),
                Panel::Closed | Panel::Details => return,
            };
            match result {
                Ok(()) => {
                    panel.set(Panel::Closed);
                    reason.set(String::new());
                    changed.run(());
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    let open_attachment = move |sha256: String| {
        // Opened by the click itself: browsers block windows opened after an await.
        let Some(tab) = window()
            .open_with_url_and_target("", "_blank")
            .ok()
            .flatten()
        else {
            return error.set(Some(describe_code("popup_blocked")));
        };
        let company = company_id.get_value();
        spawn_local(async move {
            let result = invoicing_api()
                .get_supplier_invoice_attachment(ipb::GetSupplierInvoiceAttachmentRequest {
                    company_id: company,
                    number,
                    sha256,
                })
                .await;
            match result {
                Ok(response) => {
                    let response = response.into_inner();
                    let mime = response
                        .attachment
                        .map(|a| a.content_type)
                        .unwrap_or_default();
                    open_in(&tab, &mime, &response.data);
                }
                Err(status) => {
                    let _ = tab.close();
                    error.try_set(Some(describe(&status)));
                }
            }
        });
    };

    let i = invoice.get_value();
    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>{i.supplier_name.clone()}</td>
            <td class=TABLE_CELL>{i.invoice_number.clone()}</td>
            <td class=TABLE_CELL>{i.invoice_date.clone()}</td>
            <td class=TABLE_CELL>{i.due_date.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(i.total)}</td>
            <td class=if label == "Förfallen" { format!("{TABLE_CELL} text-destructive") } else { TABLE_CELL.to_owned() }>{label}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Details)>"Detaljer"</Button>
                {unpaid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Pay)>"Betala"</Button>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Cancel)>"Makulera"</Button>
                })}
                {paid.then(|| view! {
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| toggle(Panel::Reverse)>"Ångra betalning"</Button>
                })}
            </td>
        </tr>
        <Show when=move || panel.get() != Panel::Closed>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL colspan="8">
                    {move || match panel.get() {
                        Panel::Details => {
                            let i = invoice.get_value();
                            view! {
                                <div class="grid gap-2 text-xs/relaxed">
                                    <ul class="grid gap-1">
                                        {i.lines.iter().map(|l| view! {
                                            <li>{format!("{} · {} · {} %", l.account, amount(l.net), l.vat_rate)}</li>
                                        }).collect_view()}
                                    </ul>
                                    <p>{format!("Moms {} · Att betala {}", amount(i.vat), amount(i.total))}</p>
                                    {(!i.reference.is_empty()).then(|| view! { <p>{format!("OCR/meddelande {}", i.reference)}</p> })}
                                    {(!i.bankgiro.is_empty()).then(|| view! { <p>{format!("Bankgiro {}", i.bankgiro)}</p> })}
                                    {(!i.plusgiro.is_empty()).then(|| view! { <p>{format!("Plusgiro {}", i.plusgiro)}</p> })}
                                    {(!i.iban.is_empty()).then(|| view! { <p>{format!("IBAN {}", i.iban)}</p> })}
                                    {(!i.vouchers.is_empty()).then(|| view! {
                                        <p>
                                            {i.vouchers.iter().map(|v| format!("Ver {} ({})", v.number, v.fiscal_year_start)).collect::<Vec<_>>().join(", ")}
                                            " · "
                                            <A href="/vouchers" attr:class="underline-offset-4 hover:underline">"Verifikationer"</A>
                                        </p>
                                    })}
                                    <ul class="flex flex-wrap gap-2">
                                        {i.attachments.iter().map(|a| {
                                            let sha = a.id.clone();
                                            view! {
                                                <li>
                                                    <Button variant=Variant::Ghost kind="button" on:click=move |_| open_attachment(sha.clone())>
                                                        {format!("{} ({})", a.file_name, size_label(a.size))}
                                                    </Button>
                                                </li>
                                            }
                                        }).collect_view()}
                                    </ul>
                                </div>
                            }.into_any()
                        }
                        Panel::Pay => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Betaldatum" kind="date" value=pay_date />
                                <TextInput label="Betalkonto" value=pay_account inputmode="numeric" />
                                <Button kind="button" on:click=move |_| act(Panel::Pay)>"Bekräfta betalning"</Button>
                            </div>
                        }.into_any(),
                        Panel::Cancel => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Anledning" value=reason />
                                <Button kind="button" on:click=move |_| act(Panel::Cancel)>"Bekräfta makulering"</Button>
                            </div>
                        }.into_any(),
                        Panel::Reverse => view! {
                            <div class="flex items-end gap-2">
                                <TextInput label="Anledning" value=reason />
                                <Button kind="button" on:click=move |_| act(Panel::Reverse)>"Bekräfta ångring"</Button>
                            </div>
                        }.into_any(),
                        Panel::Closed => ().into_any(),
                    }}
                </td>
            </tr>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn invoice(status: &str, due: &str) -> ipb::SupplierInvoice {
        ipb::SupplierInvoice {
            status: status.into(),
            due_date: due.into(),
            ..Default::default()
        }
    }

    #[test]
    fn an_unpaid_invoice_past_its_due_date_is_overdue() {
        assert_eq!(
            status_label(&invoice("unpaid", "2026-03-31"), "2026-03-31"),
            "Obetald"
        );
        assert_eq!(
            status_label(&invoice("unpaid", "2026-03-31"), "2026-04-01"),
            "Förfallen"
        );
        assert_eq!(
            status_label(&invoice("paid", "2026-03-31"), "2026-04-01"),
            "Betald"
        );
        assert_eq!(
            status_label(&invoice("cancelled", "2026-03-31"), "2026-04-01"),
            "Makulerad"
        );
    }
}
