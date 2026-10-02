//! The grundbok: the active company's vouchers for one fiscal year.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::{amount, today};
use crate::ui::{
    Button, ErrorAlert, SELECT_OPTION, Select, TABLE_BODY, TABLE_CELL, TABLE_HEAD,
    TABLE_HEADER_CELL, TABLE_ROW, Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

#[component]
pub fn Vouchers() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let years = RwSignal::new(Vec::<lpb::FiscalYear>::new());
    let year = RwSignal::new(String::new());
    // The vouchers with the company and fiscal year they were loaded for.
    let vouchers = RwSignal::new((
        String::new(),
        None::<lpb::FiscalYear>,
        Vec::<lpb::Voucher>::new(),
    ));
    let names = RwSignal::new(Vec::<lpb::Account>::new());
    let error = RwSignal::new(None::<String>);

    let load = move || {
        let (company_id, start) = (companies.active.get_untracked(), year.get_untracked());
        let Some(fiscal_year) =
            years.with_untracked(|ys| ys.iter().find(|y| y.start == start).cloned())
        else {
            return;
        };
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .list_vouchers(lpb::ListVouchersRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            // Company or year changed meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => vouchers.set((
                    company_id,
                    Some(fiscal_year),
                    response.into_inner().vouchers,
                )),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's data on screen.
        years.set(Vec::new());
        year.set(String::new());
        vouchers.set((String::new(), None, Vec::new()));
        names.set(Vec::new());
        error.set(None);
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let mut api = ledger_api();
            let result = api
                .list_fiscal_years(lpb::ListFiscalYearsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let accounts = api
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = accounts {
                names.set(response.into_inner().accounts);
            }
            match result {
                Ok(response) => {
                    let list = response.into_inner().fiscal_years;
                    let first = list.first().map(|y| y.start.clone()).unwrap_or_default();
                    years.set(list);
                    year.set(first);
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    Effect::new(move |_| {
        year.track();
        vouchers.set((String::new(), None, Vec::new()));
        load();
    });
    let changed = Callback::new(move |()| load());

    view! {
        <div class="grid gap-6">
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">"Verifikationer"</h1>
                <A href="/vouchers/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ny verifikation"</A>
            </div>
            <ErrorAlert message=error />
            <div class="w-56">
                <Select label="Räkenskapsår" id="fiscal_year" value=year>
                    {move || {
                        years
                            .get()
                            .into_iter()
                            .map(|y| view! { <option class=SELECT_OPTION value=y.start.clone()>{format!("{} – {}", y.start, y.end)}</option> })
                            .collect_view()
                    }}
                </Select>
            </div>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Datum"</th>
                        <th class=TABLE_HEADER_CELL>"Text"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Belopp"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, fiscal_year, list) = vouchers.get();
                            list.into_iter()
                                .map(|v| (company_id.clone(), fiscal_year.clone(), v))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, fy, v)| {
                            (company_id.clone(), fy.as_ref().map(|y| y.start.clone()), v.number, v.corrected_by)
                        }
                        let((company_id, fiscal_year, voucher))
                    >
                        <VoucherRow company_id=company_id fiscal_year=fiscal_year voucher=voucher names=names changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn VoucherRow(
    company_id: String,
    fiscal_year: Option<lpb::FiscalYear>,
    voucher: lpb::Voucher,
    names: RwSignal<Vec<lpb::Account>>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The company and year this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let fiscal_year = StoredValue::new(fiscal_year);
    let expanded = RwSignal::new(false);
    let correcting = RwSignal::new(false);
    let date = RwSignal::new(String::new());
    let number = voucher.number;
    let total: i64 = voucher.lines.iter().map(|l| l.debit).sum();
    let status = match (voucher.corrects, voucher.corrected_by) {
        (_, by) if by != 0 => format!("Rättad av ver {by}"),
        (of, _) if of != 0 => format!("Rättelse av ver {of}"),
        _ => String::new(),
    };
    let can_correct = voucher.corrects == 0 && voucher.corrected_by == 0;
    let lines = voucher.lines.clone();

    let start_correction = move |_| {
        // Today, or the year's last day once the year is over.
        let end = fiscal_year.get_value().map(|y| y.end).unwrap_or_default();
        let today = today();
        date.set(if !end.is_empty() && today > end {
            end
        } else {
            today
        });
        correcting.set(true);
    };
    let confirm = move |_| {
        error.set(None);
        let Some(year) = fiscal_year.get_value() else {
            return;
        };
        spawn_local(async move {
            let request = lpb::CorrectVoucherRequest {
                company_id: company_id.get_value(),
                fiscal_year_start: year.start,
                number,
                date: date.get_untracked(),
            };
            let result = ledger_api().correct_voucher(request).await;
            match result {
                Ok(_) => changed.run(()),
                Err(status) if company_id.get_value() == companies.active.get_untracked() => {
                    error.set(Some(describe(&status)))
                }
                Err(_) => {}
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>
                <button type="button" aria-expanded=move || expanded.get().to_string() on:click=move |_| expanded.update(|e| *e = !*e)>
                    {number}
                </button>
            </td>
            <td class=TABLE_CELL>{voucher.date.clone()}</td>
            <td class=TABLE_CELL>{voucher.text.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(total)}</td>
            <td class=TABLE_CELL>{status}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || can_correct && !correcting.get()>
                    <Button variant=Variant::Ghost kind="button" on:click=start_correction>"Rätta"</Button>
                </Show>
                <Show when=move || correcting.get()>
                    <span class="inline-flex items-center gap-2">
                        <TextInput label=format!("Datum för rättelse av ver {number}") value=date kind="date" />
                        <Button kind="button" on:click=confirm>"Bekräfta rättelse"</Button>
                    </span>
                </Show>
            </td>
        </tr>
        <Show when=move || expanded.get()>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL></td>
                <td class=TABLE_CELL colspan="5">
                    <ul class="grid gap-1">
                        {lines
                            .iter()
                            .map(|l| {
                                let name = names.with(|n| n.iter().find(|a| a.number == l.account).map(|a| a.name.clone()).unwrap_or_default());
                                let side = if l.debit > 0 {
                                    format!("Debet {}", amount(l.debit))
                                } else {
                                    format!("Kredit {}", amount(l.credit))
                                };
                                view! {
                                    <li class="flex justify-between gap-4">
                                        <span>{format!("{} {}", l.account, name)}</span>
                                        <span class="tabular-nums">{side}</span>
                                    </li>
                                }
                            })
                            .collect_view()}
                    </ul>
                </td>
            </tr>
        </Show>
    }
}
