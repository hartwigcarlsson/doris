//! Book a voucher in the active company. The server decides its number.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::today;
use crate::ui::{Button, Card, ErrorAlert, Field};
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
    let error = RwSignal::new(None::<String>);
    let booked = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    // The company this form was filled for; a submit only ever goes there.
    let form_company = StoredValue::new(String::new());
    let clear = move || {
        text.set(String::new());
        lines.clear();
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
        let Some(request_lines) = lines.request() else {
            return error.set(Some("Skriv beloppen som 1 234,50.".into()));
        };
        busy.set(true);
        let company_id = form_company.get_value();
        spawn_local(async move {
            let request = lpb::RecordVoucherRequest {
                company_id: company_id.clone(),
                date: date.get_untracked(),
                text: text.get_untracked(),
                lines: request_lines,
                attachments: Vec::new(),
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
                <Button disabled=busy>"Bokför"</Button>
            </form>
        </Card>
    }
}
