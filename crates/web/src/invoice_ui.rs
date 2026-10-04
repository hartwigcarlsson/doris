//! Pieces the supplier and customer invoice pages share: the status label,
//! the line editor with its VAT preview, the picked underlag, and the
//! pay and reason forms.

use crate::api::{ipb, lpb};
use crate::attachments::{read_files, size_label};
use crate::errors::describe_code;
use crate::format::parse_amount;
use crate::ui::{Button, FileInput, SELECT, SELECT_OPTION, TextInput, Variant};
use crate::voucher_lines::account_number;
use leptos::prelude::*;
use leptos::task::spawn_local;
use std::collections::BTreeMap;

/// Obetald, Förfallen (unpaid past its due date), Betald or Makulerad.
pub fn status_label(status: &str, due_date: &str, today: &str) -> &'static str {
    match status {
        "paid" => "Betald",
        "cancelled" => "Makulerad",
        _ if due_date < today => "Förfallen",
        _ => "Obetald",
    }
}

/// VAT per rate on that rate's summed net, rounded half up, highest rate
/// first and only rates with VAT: the server's rule, shown while typing.
pub fn preview_vat(lines: &[(i64, u32)]) -> Vec<(u32, i64)> {
    let mut by_rate = BTreeMap::<u32, i64>::new();
    for &(net, rate) in lines {
        *by_rate.entry(rate).or_default() += net;
    }
    by_rate
        .into_iter()
        .rev()
        .map(|(rate, net)| (rate, (net * i64::from(rate) + 50) / 100))
        .filter(|&(_, vat)| vat > 0)
        .collect()
}

/// One line in the editor: Konto, Belopp exkl. moms, Momssats.
#[derive(Clone, Copy)]
pub struct LineRow {
    pub id: u32,
    pub account: RwSignal<String>,
    pub net: RwSignal<String>,
    pub rate: RwSignal<String>,
}

impl LineRow {
    pub fn new(id: u32) -> Self {
        Self {
            id,
            account: RwSignal::new(String::new()),
            net: RwSignal::new(String::new()),
            rate: RwSignal::new("25".into()),
        }
    }

    /// (net in öre, rate) as typed; an unreadable amount counts as 0.
    pub fn preview(&self) -> (i64, u32) {
        (
            parse_amount(&self.net.get()).unwrap_or(0),
            self.rate.get().parse().unwrap_or(25),
        )
    }

    pub fn request(&self) -> Option<ipb::InvoiceLine> {
        Some(ipb::InvoiceLine {
            account: account_number(&self.account.get_untracked()),
            net: parse_amount(&self.net.get_untracked())?,
            vat_rate: self.rate.get_untracked().parse().unwrap_or(25),
        })
    }
}

/// The line editor. `list` is the id of the page's account `<datalist>`.
#[component]
pub fn InvoiceLineRows(
    rows: RwSignal<Vec<LineRow>>,
    next_id: StoredValue<u32>,
    list: &'static str,
) -> impl IntoView {
    view! {
        <div class="grid gap-2">
            <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2 text-muted-foreground">
                <span>"Konto"</span>
                <span>"Belopp exkl. moms"</span>
                <span>"Moms"</span>
                <span></span>
            </div>
            <For each=move || { rows.get().into_iter().enumerate().collect::<Vec<_>>() } key=|(i, r)| (*i, r.id) let((index, row))>
                <div class="grid grid-cols-[1fr_8rem_6rem_auto] gap-2">
                    <TextInput label=format!("Konto, rad {}", index + 1) value=row.account list=list />
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
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| rows.update(|all| all.retain(|other| other.id != row.id))>
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
                        rows.update(|all| all.push(LineRow::new(id)));
                    }
                >
                    "Lägg till rad"
                </Button>
            </div>
        </div>
    }
}

/// The Underlag picker and the picked files, each removable. `reading`
/// counts picks still being read. Picks that finish after the form has
/// moved to another `company` are dropped.
#[component]
pub fn PickedFiles(
    id: &'static str,
    files: RwSignal<Vec<lpb::NewAttachment>>,
    reading: RwSignal<u32>,
    error: RwSignal<Option<String>>,
    company: StoredValue<String>,
) -> impl IntoView {
    let pick = move |input: web_sys::HtmlInputElement| {
        let company_id = company.get_value();
        reading.update(|n| *n += 1);
        spawn_local(async move {
            let picked = read_files(&input).await;
            reading.try_update(|n| *n -= 1);
            if company_id != company.get_value() {
                return;
            }
            match picked {
                Ok(picked) => files.update(|f| f.extend(picked)),
                Err(code) => error.set(Some(describe_code(code))),
            }
        });
    };
    view! {
        <div class="grid gap-2">
            <FileInput label="Underlag" id=id on_pick=pick />
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
    }
}

/// Betaldatum and Betalkonto, and a button that confirms.
#[component]
pub fn PayForm(
    date: RwSignal<String>,
    account: RwSignal<String>,
    confirm: &'static str,
    on_confirm: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="flex items-end gap-2">
            <TextInput label="Betaldatum" kind="date" value=date />
            <TextInput label="Betalkonto" value=account inputmode="numeric" />
            <Button kind="button" on:click=move |_| on_confirm.run(())>{confirm}</Button>
        </div>
    }
}

/// Anledning, and a button that confirms.
#[component]
pub fn ReasonForm(
    reason: RwSignal<String>,
    confirm: &'static str,
    on_confirm: Callback<()>,
) -> impl IntoView {
    view! {
        <div class="flex items-end gap-2">
            <TextInput label="Anledning" value=reason />
            <Button kind="button" on:click=move |_| on_confirm.run(())>{confirm}</Button>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn an_unpaid_invoice_past_its_due_date_is_overdue() {
        assert_eq!(
            status_label("unpaid", "2026-03-31", "2026-03-31"),
            "Obetald"
        );
        assert_eq!(
            status_label("unpaid", "2026-03-31", "2026-04-01"),
            "Förfallen"
        );
        assert_eq!(status_label("paid", "2026-03-31", "2026-04-01"), "Betald");
        assert_eq!(
            status_label("cancelled", "2026-03-31", "2026-04-01"),
            "Makulerad"
        );
    }

    #[test]
    fn the_preview_rounds_vat_per_rate_like_the_server() {
        assert_eq!(preview_vat(&[(33, 25), (33, 25), (33, 25)]), [(25, 25)]);
        assert_eq!(
            preview_vat(&[(1000, 12), (50, 6), (700, 0)]),
            [(12, 120), (6, 3)]
        );
        assert!(preview_vat(&[]).is_empty());
    }
}
