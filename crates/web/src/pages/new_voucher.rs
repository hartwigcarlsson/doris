//! Book a voucher in the active company. The server decides its number.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::format::{amount, parse_amount, today};
use crate::ui::{Button, Card, ErrorAlert, Field, TextInput, Variant};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The account number at the start of what was typed or picked from the
/// list ("1930 Företagskonto" → 1930). 0 when there is none; the server
/// refuses it as an unknown account.
pub fn account_number(raw: &str) -> u32 {
    raw.split_whitespace()
        .next()
        .and_then(|n| n.parse().ok())
        .unwrap_or(0)
}

/// An empty amount field is 0; anything else must parse.
fn field_amount(raw: &str) -> Option<i64> {
    if raw.trim().is_empty() {
        Some(0)
    } else {
        parse_amount(raw)
    }
}

#[derive(Clone, Copy)]
struct Line {
    id: usize,
    account: RwSignal<String>,
    debit: RwSignal<String>,
    credit: RwSignal<String>,
}

impl Line {
    fn new(id: usize) -> Self {
        Self {
            id,
            account: RwSignal::new(String::new()),
            debit: RwSignal::new(String::new()),
            credit: RwSignal::new(String::new()),
        }
    }

    fn is_blank(&self) -> bool {
        [self.account, self.debit, self.credit]
            .iter()
            .all(|s| s.get_untracked().trim().is_empty())
    }
}

#[component]
pub fn NewVoucher() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let accounts = RwSignal::new(Vec::<lpb::Account>::new());
    let date = RwSignal::new(today());
    let text = RwSignal::new(String::new());
    let next_id = StoredValue::new(2_usize);
    let lines = RwSignal::new(vec![Line::new(0), Line::new(1)]);
    let error = RwSignal::new(None::<String>);
    let booked = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    // The company this form was filled for; a submit only ever goes there.
    let form_company = StoredValue::new(String::new());
    let clear = move || {
        text.set(String::new());
        let id = next_id.get_value();
        next_id.set_value(id + 2);
        lines.set(vec![Line::new(id), Line::new(id + 1)]);
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

    let add_line = move |_| {
        let id = next_id.get_value();
        next_id.set_value(id + 1);
        lines.update(|l| l.push(Line::new(id)));
    };
    let totals = move || {
        lines.with(|l| {
            l.iter().fold((0_i64, 0_i64), |(d, c), line| {
                (
                    d + line.debit.with(|s| field_amount(s)).unwrap_or(0),
                    c + line.credit.with(|s| field_amount(s)).unwrap_or(0),
                )
            })
        })
    };

    let submit = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        booked.set(None);
        let mut request_lines = Vec::new();
        for line in lines.get_untracked().iter().filter(|l| !l.is_blank()) {
            let (Some(debit), Some(credit)) = (
                field_amount(&line.debit.get_untracked()),
                field_amount(&line.credit.get_untracked()),
            ) else {
                return error.set(Some("Skriv beloppen som 1 234,50.".into()));
            };
            request_lines.push(lpb::VoucherLine {
                account: account_number(&line.account.get_untracked()),
                debit,
                credit,
            });
        }
        busy.set(true);
        let company_id = form_company.get_value();
        spawn_local(async move {
            let request = lpb::RecordVoucherRequest {
                company_id: company_id.clone(),
                date: date.get_untracked(),
                text: text.get_untracked(),
                lines: request_lines,
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
                <div class="grid gap-2">
                    <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2 text-muted-foreground">
                        <span>"Konto"</span>
                        <span>"Debet"</span>
                        <span>"Kredit"</span>
                        <span></span>
                    </div>
                    <For each=move || { lines.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, l)| (*i, l.id) let((index, line))>
                        <div class="grid grid-cols-[1fr_8rem_8rem_auto] gap-2">
                            <TextInput label=format!("Konto, rad {}", index + 1) value=line.account list="accounts" />
                            <TextInput label=format!("Debet, rad {}", index + 1) value=line.debit inputmode="decimal" />
                            <TextInput label=format!("Kredit, rad {}", index + 1) value=line.credit inputmode="decimal" />
                            <Button
                                variant=Variant::Ghost
                                kind="button"
                                on:click=move |_| lines.update(|l| l.retain(|other| other.id != line.id))
                            >
                                "Ta bort"
                            </Button>
                        </div>
                    </For>
                    <div>
                        <Button variant=Variant::Ghost kind="button" on:click=add_line>"Lägg till rad"</Button>
                    </div>
                </div>
                <p class="text-xs/relaxed text-muted-foreground">
                    {move || {
                        let (debit, credit) = totals();
                        format!("Debet {} · Kredit {} · Differens {}", amount(debit), amount(credit), amount(debit - credit))
                    }}
                </p>
                <Button disabled=busy>"Bokför"</Button>
            </form>
        </Card>
    }
}

#[cfg(test)]
mod tests {
    use super::account_number;

    #[test]
    fn account_number_takes_the_leading_digits() {
        assert_eq!(account_number("1930"), 1930);
        assert_eq!(
            account_number(" 1930 Företagskonto/checkkonto/affärskonto"),
            1930
        );
        assert_eq!(account_number("Företagskonto"), 0);
        assert_eq!(account_number(""), 0);
        assert_eq!(account_number("19x0"), 0);
    }
}
