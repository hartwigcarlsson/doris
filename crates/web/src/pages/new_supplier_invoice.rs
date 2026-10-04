//! Register a supplier invoice in the active company. The server books it
//! and decides its number.

use crate::active_company::Companies;
use crate::api::{invoicing_api, ipb, ledger_api, lpb};
use crate::attachments::{check_sizes, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::format::{amount, parse_amount, plus_days, today};
use crate::ui::{
    Button, Card, ErrorAlert, Field, FileInput, SELECT, SELECT_OPTION, Select, TextInput, Variant,
};
use crate::voucher_lines::account_number;
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::hooks::use_navigate;

/// VAT per rate on that rate's summed net, rounded half up: the server's
/// rule, shown while typing. The server decides.
fn preview_vat(lines: &[(i64, u32)]) -> i64 {
    let mut by_rate = std::collections::BTreeMap::<u32, i64>::new();
    for &(net, rate) in lines {
        *by_rate.entry(rate).or_default() += net;
    }
    by_rate
        .into_iter()
        .map(|(rate, net)| (net * i64::from(rate) + 50) / 100)
        .sum()
}

#[derive(Clone, Copy)]
struct Row {
    id: u32,
    account: RwSignal<String>,
    net: RwSignal<String>,
    rate: RwSignal<String>,
}

impl Row {
    fn new(id: u32) -> Self {
        Self {
            id,
            account: RwSignal::new(String::new()),
            net: RwSignal::new(String::new()),
            rate: RwSignal::new("25".into()),
        }
    }

    /// (net in öre, rate) as typed; an unreadable amount counts as 0.
    fn preview(&self) -> (i64, u32) {
        (
            parse_amount(&self.net.get()).unwrap_or(0),
            self.rate.get().parse().unwrap_or(25),
        )
    }

    fn request(&self) -> Option<ipb::InvoiceLine> {
        Some(ipb::InvoiceLine {
            account: account_number(&self.account.get_untracked()),
            net: parse_amount(&self.net.get_untracked())?,
            vat_rate: self.rate.get_untracked().parse().unwrap_or(25),
        })
    }
}

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
    let rows = RwSignal::new(vec![Row::new(0)]);
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
        let lines: Vec<(i64, u32)> = rows.get().iter().map(Row::preview).collect();
        let computed = amount(preview_vat(&lines));
        if vat.get_untracked() == auto_vat.get_untracked() {
            vat.set(computed.clone());
        }
        auto_vat.set(computed);
    });
    Effect::new(move |_| {
        let company_id = companies.active.get();
        suppliers.set(Vec::new());
        accounts.set(Vec::new());
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

    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = form_company.get_value();
        reading.update(|n| *n += 1);
        spawn_local(async move {
            let picked = read_files(&input).await;
            reading.try_update(|n| *n -= 1);
            if company_id != form_company.get_value() {
                return;
            }
            match picked {
                Ok(picked) => files.update(|f| f.extend(picked)),
                Err(code) => error.set(Some(describe_code(code))),
            }
        });
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        if reading.get_untracked() > 0 {
            return;
        }
        let Some(lines) = rows
            .get_untracked()
            .iter()
            .map(Row::request)
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
        <Card title="Ny leverantörsfaktura">
            <form class="grid gap-4" data-wide novalidate on:submit=submit>
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
                <div class="grid gap-2">
                    <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2 text-muted-foreground">
                        <span>"Konto"</span>
                        <span>"Belopp exkl. moms"</span>
                        <span>"Moms"</span>
                        <span></span>
                    </div>
                    <For each=move || { rows.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, r)| (*i, r.id) let((index, row))>
                        <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2">
                            <TextInput label=format!("Konto, rad {}", index + 1) value=row.account list="invoice_accounts" />
                            <TextInput label=format!("Belopp exkl. moms, rad {}", index + 1) value=row.net inputmode="decimal" />
                            <select
                                class=SELECT
                                aria-label=format!("Momssats, rad {}", index + 1)
                                prop:value=move || row.rate.get()
                                on:change=move |ev| row.rate.set(event_target_value(&ev))
                            >
                                <option class=SELECT_OPTION value="25">"25 %"</option>
                                <option class=SELECT_OPTION value="12">"12 %"</option>
                                <option class=SELECT_OPTION value="6">"6 %"</option>
                                <option class=SELECT_OPTION value="0">"0 %"</option>
                            </select>
                            <Button
                                variant=Variant::Ghost
                                kind="button"
                                on:click=move |_| rows.update(|all| all.retain(|other| other.id != row.id))
                            >
                                "Ta bort"
                            </Button>
                        </div>
                    </For>
                    <div>
                        <Button
                            variant=Variant::Ghost
                            kind="button"
                            on:click=move |_| {
                                let id = next_id.get_value();
                                next_id.set_value(id + 1);
                                rows.update(|all| all.push(Row::new(id)));
                            }
                        >
                            "Lägg till rad"
                        </Button>
                    </div>
                </div>
                <div class="grid grid-cols-[10rem_1fr] items-end gap-4">
                    <Field label="Moms" id="invoice_vat" value=vat />
                    <p class="text-xs/relaxed">
                        {move || format!("Netto {} · Att betala {}", amount(net_total()), amount(to_pay()))}
                    </p>
                </div>
                <div class="grid gap-2">
                    <FileInput label="Underlag" id="invoice_files" on_pick=pick />
                    <ul class="grid gap-1">
                        {move || {
                            files.with(|picked| {
                                picked
                                    .iter()
                                    .enumerate()
                                    .map(|(i, f)| {
                                        let name = f.file_name.clone();
                                        view! {
                                            <li class="flex items-center justify-between gap-4 text-xs/relaxed">
                                                <span>{format!("{} ({})", name, size_label(f.data.len() as u64))}</span>
                                                <Button
                                                    variant=Variant::Ghost
                                                    kind="button"
                                                    attr:aria-label=format!("Ta bort {name}")
                                                    on:click=move |_| files.update(|f| { f.remove(i); })
                                                >
                                                    "Ta bort"
                                                </Button>
                                            </li>
                                        }
                                    })
                                    .collect_view()
                            })
                        }}
                    </ul>
                </div>
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>"Registrera"</Button>
            </form>
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::preview_vat;

    #[test]
    fn the_preview_rounds_vat_per_rate_like_the_server() {
        assert_eq!(preview_vat(&[(33, 25), (33, 25), (33, 25)]), 25);
        assert_eq!(preview_vat(&[(1000, 12), (50, 6), (700, 0)]), 123);
        assert_eq!(preview_vat(&[]), 0);
    }
}
