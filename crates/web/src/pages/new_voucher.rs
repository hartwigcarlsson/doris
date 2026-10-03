//! Book a voucher in the active company. The server decides its number.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::attachments::{check_sizes, read_files, size_label};
use crate::errors::{describe, describe_code};
use crate::format::today;
use crate::ui::{Button, Card, ErrorAlert, Field, FileInput, Variant};
use crate::voucher_lines::{LineRows, Lines};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

#[component]
pub fn NewVoucher() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let date = RwSignal::new(today());
    let text = RwSignal::new(String::new());
    let lines = Lines::new();
    // Read into memory when picked; they go up with the voucher.
    let files = RwSignal::new(Vec::<lpb::NewAttachment>::new());
    let error = RwSignal::new(None::<String>);
    let booked = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    // Picks still being read; booking waits for them.
    let reading = RwSignal::new(0u32);
    // The company this form was filled for; a submit only ever goes there.
    let form_company = StoredValue::new(String::new());
    let clear = move || {
        text.set(String::new());
        lines.clear();
        files.set(Vec::new());
    };
    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = form_company.get_value();
        reading.update(|n| *n += 1);
        spawn_local(async move {
            let picked = read_files(&input).await;
            reading.try_update(|n| *n -= 1);
            // Picked for a company that is no longer the form's: drop them.
            if company_id != form_company.get_value() {
                return;
            }
            match picked {
                Ok(picked) => files.update(|f| f.extend(picked)),
                Err(code) => error.set(Some(describe_code(code))),
            }
        });
    };

    Effect::new(move |_| {
        let company_id = companies.active.get();
        // The list must be the active company's: never offer another's
        // accounts, nor keep lines typed for it.
        accounts.set(Vec::new());
        error.set(None);
        booked.set(None);
        clear();
        form_company.set_value(company_id.clone());
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = ledger_api()
                .list_accounts(lpb::ListAccountsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            if let Ok(response) = result {
                accounts.set(response.into_inner().accounts);
            }
        });
    });

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        booked.set(None);
        if reading.get_untracked() > 0 {
            return;
        }
        let Some(request_lines) = lines.request() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        if let Err(code) = files.with_untracked(|f| check_sizes(f)) {
            return error.set(Some(describe_code(code)));
        }
        busy.set(true);
        let company_id = form_company.get_value();
        spawn_local(async move {
            let request = lpb::RecordVoucherRequest {
                company_id: company_id.clone(),
                date: date.get_untracked(),
                text: text.get_untracked(),
                lines: request_lines,
                attachments: files.get_untracked(),
            };
            let result = ledger_api().record_voucher(request).await;
            busy.set(false);
            // Switched company meanwhile: say nothing about the other company.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    booked.set(Some(format!(
                        "Verifikation {} bokförd",
                        response.into_inner().number
                    )));
                    clear();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <Card title="Ny verifikation">
            <form class="grid gap-4" data-wide novalidate on:submit=submit>
                <ErrorAlert message=error />
                {move || booked.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
                <div class="grid grid-cols-[10rem_1fr] gap-4">
                    <Field label="Datum" id="voucher_date" kind="date" value=date />
                    <Field label="Text" id="voucher_text" value=text />
                </div>
                <div class="grid gap-2">
                    <FileInput label="Underlag" id="voucher_files" on_pick=pick />
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
                <datalist id="accounts">
                    {move || {
                        accounts
                            .get()
                            .into_iter()
                            .filter(|a| a.active)
                            .map(|a| view! { <option value=format!("{} {}", a.number, a.name) /> })
                            .collect_view()
                    }}
                </datalist>
                <LineRows lines=lines list="accounts" />
                <Button disabled=Signal::derive(move || busy.get() || reading.get() > 0)>
                    "Bokför"
                </Button>
            </form>
        </Card>
    }
}
