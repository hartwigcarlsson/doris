//! The grundbok: the active company's vouchers for one fiscal year.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::attachments::{open_in, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::fiscal_year::is_closed;
use crate::format::{amount, local_time, today};
use crate::ui::{
    Badge, Button, Checkbox, ErrorAlert, FileInput, INPUT, Icon, IconName, LinkButton, PageHeader,
    SELECT_OPTION, Select, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table,
    TableCard, TextInput, Variant,
};
use crate::voucher_search::{Filter, PAGE, shown, visible};
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
        // `try_`: a row may ask for a reload after the page is gone.
        let Some(start) = year.try_get_untracked() else {
            return;
        };
        let company_id = companies.active.get_untracked();
        let Some(fiscal_year) = years
            .try_with_untracked(|ys| ys.iter().find(|y| y.start == start).cloned())
            .flatten()
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
            // `try_`: the page may be gone, and its signals with it.
            if company_id != companies.active.get_untracked()
                || year.try_get_untracked().as_deref() != Some(start.as_str())
            {
                return;
            }
            match result {
                Ok(response) => vouchers.set((
                    company_id,
                    Some(fiscal_year),
                    response.into_inner().vouchers,
                )),
                Err(status) => {
                    error.try_set(Some(describe(&status)));
                }
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
        error.set(None);
        load();
    });
    let changed = Callback::new(move |()| load());

    let query = RwSignal::new(String::new());
    let missing = RwSignal::new(false);
    let corrections = RwSignal::new(false);
    let limit = RwSignal::new(PAGE);
    let filter = Memo::new(move |_| Filter {
        query: query.get(),
        missing_attachment: missing.get(),
        corrections: corrections.get(),
    });
    // A new question, year or company starts from the top.
    Effect::new(move |_| {
        filter.track();
        year.track();
        limit.set(PAGE);
    });
    // (company, fiscal year, vouchers) as loaded; the list shows the
    // matching ones, highest number first.
    let matching = Memo::new(move |_| {
        let (company_id, fiscal_year, mut list) = vouchers.get();
        let filter = filter.get();
        names.with(|accounts| list.retain(|v| visible(v, accounts, &filter)));
        list.sort_by_key(|v| std::cmp::Reverse(v.number));
        (company_id, fiscal_year, list)
    });

    view! {
        <div class="grid gap-6">
            <PageHeader title="Verifikationer">
                <Show when=move || years.with(|ys| is_closed(ys, &year.get()))>
                    <Badge>"Stängt"</Badge>
                </Show>
                <div class="w-56">
                    <Select label="Räkenskapsår" id="fiscal_year" hide_label=true value=year>
                        {move || {
                            years
                                .get()
                                .into_iter()
                                .map(|y| view! { <option class=SELECT_OPTION value=y.start.clone()>{format!("{} – {}", y.start, y.end)}</option> })
                                .collect_view()
                        }}
                    </Select>
                </div>
                <LinkButton href="/vouchers/new" icon=IconName::Plus>"Ny verifikation"</LinkButton>
            </PageHeader>
            <ErrorAlert message=error />
            <TableCard toolbar=move || view! {
                <div class="relative max-w-xs flex-[1_1_15rem]">
                    <Icon name=IconName::Search class="pointer-events-none absolute top-1/2 left-2 size-3.5 -translate-y-1/2 text-muted-foreground" />
                    <input
                        type="search"
                        aria-label="Sök bland verifikationer"
                        placeholder="Sök nummer, text, konto eller belopp"
                        class=format!("{INPUT} pl-7")
                        bind:value=query
                    />
                </div>
                <Checkbox label="Saknar underlag" id="missing_attachment" checked=missing />
                <Checkbox label="Rättelser" id="corrections" checked=corrections />
            }>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Nr"</th>
                        <th class=TABLE_HEADER_CELL>"Datum"</th>
                        <th class=TABLE_HEADER_CELL>"Text"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Belopp"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL><span class="sr-only">"Underlag"</span></th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, fiscal_year, list) = matching.get();
                            list.into_iter()
                                .take(limit.get())
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
            {move || {
                // The year is set once its vouchers are in: nothing to say before.
                let loaded = vouchers.with(|(_, fiscal_year, _)| fiscal_year.is_some());
                let in_year = vouchers.with(|(_, _, list)| list.len());
                let total = matching.with(|(_, _, list)| list.len());
                let showing = shown(total, limit.get());
                loaded.then(|| view! {
                    <div class="flex flex-wrap items-center justify-between gap-4 px-2 pt-2 pb-1">
                        {if in_year == 0 {
                            view! { <p class="text-muted-foreground">"Inga verifikationer under räkenskapsåret."</p> }.into_any()
                        } else if total == 0 {
                            view! { <p class="text-muted-foreground">"Inga verifikationer matchar."</p> }.into_any()
                        } else {
                            view! { <p role="status" class="text-muted-foreground">{format!("Visar {showing} av {total}")}</p> }.into_any()
                        }}
                        {(showing < total).then(|| view! {
                            <Button variant=Variant::Outline kind="button" on:click=move |_| limit.update(|l| *l += PAGE)>"Visa fler"</Button>
                        })}
                    </div>
                })
            }}
            </TableCard>
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
    let closed = fiscal_year.as_ref().is_some_and(|y| y.closed);
    // The huvudbok links go to the year this row was loaded for.
    let ledger_year = fiscal_year
        .as_ref()
        .map(|y| y.start.clone())
        .unwrap_or_default();
    let fiscal_year = StoredValue::new(fiscal_year);
    // Behandlingshistorik: when it was recorded, in this browser's time
    // zone (JS counts minutes west of UTC), and by whom if known.
    // The offset is the one that applied then, not today's: summer time
    // must not move an old voucher an hour. (An unreadable time gives NaN,
    // which is 0 here, and `local_time` shows nothing for it anyway.)
    let then = js_sys::Date::new(&wasm_bindgen::JsValue::from_str(&voucher.recorded_at));
    let offset = -(then.get_timezone_offset() as i32);
    let recorded = local_time(&voucher.recorded_at, offset);
    let history = match (recorded.is_empty(), voucher.recorded_by_name.is_empty()) {
        (true, _) => None,
        (false, true) => Some(format!("Bokförd {recorded}")),
        (false, false) => Some(format!(
            "Bokförd {recorded} av {}",
            voucher.recorded_by_name
        )),
    };
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
    // A closed year takes no correction; the server refuses one anyway.
    let can_correct = voucher.corrects == 0 && voucher.corrected_by == 0 && !closed;
    let lines = voucher.lines.clone();
    let attachments = RwSignal::new(voucher.attachments.clone());
    let open_attachment = move |id: String| {
        error.set(None);
        let Some(year) = fiscal_year.get_value() else {
            return;
        };
        // Opened by the click itself: browsers block window.open after an await.
        let Some(tab) = window()
            .open_with_url_and_target("", "_blank")
            .ok()
            .flatten()
        else {
            return error.set(Some(describe_code("popup_blocked")));
        };
        let company = company_id.get_value();
        spawn_local(async move {
            let result = ledger_api()
                .get_attachment(lpb::GetAttachmentRequest {
                    company_id: company.clone(),
                    fiscal_year_start: year.start,
                    number,
                    id,
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
                    if company == companies.active.get_untracked() {
                        error.try_set(Some(describe(&status)));
                    }
                }
            }
        });
    };
    // One file per request; also in a closed year.
    let add_attachments = move |input: web_sys::HtmlInputElement| {
        error.set(None);
        let Some(year) = fiscal_year.get_value() else {
            return;
        };
        let company = company_id.get_value();
        spawn_local(async move {
            let picked = match read_files(&input).await {
                Ok(picked) => picked,
                Err(code) => {
                    error.try_set(Some(describe_code(code)));
                    return;
                }
            };
            for file in picked {
                let result = ledger_api()
                    .add_attachment(lpb::AddAttachmentRequest {
                        company_id: company.clone(),
                        fiscal_year_start: year.start.clone(),
                        number,
                        attachment: Some(file),
                    })
                    .await;
                match result {
                    Ok(response) => {
                        if let Some(added) = response.into_inner().attachment {
                            // The row is gone (company or year switched): stop.
                            if attachments.try_update(|list| list.push(added)).is_none() {
                                return;
                            }
                        }
                    }
                    Err(status) => {
                        if company == companies.active.get_untracked() {
                            error.try_set(Some(describe(&status)));
                        }
                        return;
                    }
                }
            }
            // The list the search and the filters read has the new underlag
            // too; the row keeps its key, so it stays as it is.
            changed.try_run(());
        });
    };

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
            let company = request.company_id.clone();
            let result = ledger_api().correct_voucher(request).await;
            // The page may be gone by now: `try_` on everything it owns.
            match result {
                Ok(_) => {
                    changed.try_run(());
                }
                Err(status) if company == companies.active.get_untracked() => {
                    error.try_set(Some(describe(&status)));
                }
                Err(_) => {}
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>
                <button
                    type="button"
                    class="inline-flex h-7 items-center gap-1 rounded-md pr-1.5 font-medium tabular-nums hover:bg-muted"
                    aria-expanded=move || expanded.get().to_string()
                    on:click=move |_| expanded.update(|e| *e = !*e)
                >
                    {move || view! { <Icon name=if expanded.get() { IconName::ChevronDown } else { IconName::ChevronRight } class="size-3.5 text-muted-foreground" /> }}
                    {number}
                </button>
            </td>
            <td class=TABLE_CELL>{voucher.date.clone()}</td>
            <td class=TABLE_CELL>{voucher.text.clone()}</td>
            <td class=format!("{TABLE_CELL} text-right tabular-nums")>{amount(total)}</td>
            <td class=TABLE_CELL>{(!status.is_empty()).then(|| view! { <Badge>{status}</Badge> })}</td>
            <td class=TABLE_CELL>
                {move || {
                    let count = attachments.with(Vec::len);
                    (count > 0)
                        .then(|| view! {
                            <span class="inline-flex items-center gap-1 text-muted-foreground">
                                <Icon name=IconName::Paperclip />
                                {count}
                                <span class="sr-only">" underlag"</span>
                            </span>
                        })
                }}
            </td>
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
            <tr class=format!("{TABLE_ROW} bg-muted/50")>
                <td class=TABLE_CELL></td>
                <td class=TABLE_CELL colspan="6">
                    <div class="flex flex-wrap items-start gap-x-8 gap-y-3">
                    <table class="w-full max-w-xl text-xs">
                        <thead>
                            // `!`: the outer table clears the border of every last row.
                            <tr class="border-b! text-muted-foreground">
                                <th class="py-1 pr-2 text-left font-normal">"Konto"</th>
                                <th class="px-2 py-1 text-right font-normal">"Debet"</th>
                                <th class="py-1 pl-2 text-right font-normal">"Kredit"</th>
                            </tr>
                        </thead>
                        <tbody>
                            {lines
                                .iter()
                                .map(|l| {
                                    let name = names.with(|n| n.iter().find(|a| a.number == l.account).map(|a| a.name.clone()).unwrap_or_default());
                                    view! {
                                        <tr class="border-b">
                                            <td class="py-1.5 pr-2">
                                                <A href=format!("/trial-balance/{}?fy={ledger_year}", l.account) attr:class="underline-offset-4 hover:underline">
                                                    {format!("{} {}", l.account, name)}
                                                </A>
                                            </td>
                                            <td class="px-2 py-1.5 text-right tabular-nums">{(l.debit > 0).then(|| amount(l.debit))}</td>
                                            <td class="py-1.5 pl-2 text-right tabular-nums">{(l.credit > 0).then(|| amount(l.credit))}</td>
                                        </tr>
                                    }
                                })
                                .collect_view()}
                            <tr class="font-medium">
                                <td class="py-1.5 pr-2">"Summa"</td>
                                <td class="px-2 py-1.5 text-right tabular-nums">{amount(total)}</td>
                                <td class="py-1.5 pl-2 text-right tabular-nums">{amount(total)}</td>
                            </tr>
                        </tbody>
                    </table>
                    <div class="grid gap-3">
                    <div class="grid gap-2">
                        <h2 class="text-xs/relaxed font-medium">"Underlag"</h2>
                        <ul class="grid gap-1">
                            {move || {
                                attachments
                                    .get()
                                    .into_iter()
                                    .map(|a| {
                                        let label = format!("{} ({})", a.file_name, size_label(a.size));
                                        view! {
                                            <li>
                                                <button
                                                    type="button"
                                                    class="underline-offset-4 hover:underline"
                                                    on:click=move |_| open_attachment(a.id.clone())
                                                >
                                                    {label}
                                                </button>
                                            </li>
                                        }
                                    })
                                    .collect_view()
                            }}
                        </ul>
                        <div class="w-72">
                            <FileInput
                                label=format!("Lägg till underlag till ver {number}")
                                id=format!("attach_{number}")
                                on_pick=add_attachments
                            />
                        </div>
                    </div>
                    {history.clone().map(|line| view! {
                        <div class="grid gap-1">
                            <h2 class="text-xs/relaxed font-medium">"Behandlingshistorik"</h2>
                            <p>{line}</p>
                        </div>
                    })}
                    </div>
                    </div>
                </td>
            </tr>
        </Show>
    }
}
