//! Arbetsgivardeklaration (AGI): the contact person, each month's
//! individuppgifter and huvuduppgift, the file for Skatteverket and
//! marking a month submitted.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::app::Session;
use crate::attachments::save_as;
use crate::errors::describe;
use crate::format::amount;
use crate::ui::{
    Button, Card, ErrorAlert, Field, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD,
    TABLE_HEADER_CELL, TABLE_ROW, Table, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

const MONTHS: [&str; 12] = [
    "januari",
    "februari",
    "mars",
    "april",
    "maj",
    "juni",
    "juli",
    "augusti",
    "september",
    "oktober",
    "november",
    "december",
];

/// "202610" → "oktober 2026"; anything else as it is.
pub fn period_label(period: &str) -> String {
    let month = period.get(4..6).and_then(|m| m.parse::<usize>().ok());
    match (period.get(..4), month) {
        (Some(year), Some(m @ 1..=12)) if period.len() == 6 => format!("{} {year}", MONTHS[m - 1]),
        _ => period.to_owned(),
    }
}

pub fn agi_status_label(status: ppb::AgiStatus) -> &'static str {
    match status {
        ppb::AgiStatus::NotSubmitted => "Ej deklarerad",
        ppb::AgiStatus::Submitted => "Deklarerad",
        ppb::AgiStatus::Changed => "Ändrad",
        ppb::AgiStatus::Unspecified => "",
    }
}

pub fn agi_change_label(change: ppb::AgiChange) -> &'static str {
    match change {
        ppb::AgiChange::New => "Ny",
        ppb::AgiChange::Changed => "Ändrad",
        ppb::AgiChange::Removed => "Borttag",
        ppb::AgiChange::Unchanged | ppb::AgiChange::Unspecified => "–",
    }
}

/// Whole kronor as "35 000,00".
fn kronor(kr: i64) -> String {
    amount(kr * 100)
}

#[component]
pub fn Agi() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let session = expect_context::<Session>();
    // The company the page was loaded for, not whatever is active now.
    let company = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let phone = RwSignal::new(String::new());
    let email = RwSignal::new(String::new());
    let has_contact = RwSignal::new(false);
    let months = RwSignal::new(Vec::<ppb::AgiMonthSummary>::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let contact = payroll_api()
                .get_agi_contact(ppb::GetAgiContactRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let list = payroll_api()
                .list_agi_months(ppb::ListAgiMonthsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match (contact, list) {
                (Ok(contact), Ok(list)) => {
                    let contact = contact.into_inner();
                    has_contact.set(!contact.name.is_empty());
                    if contact.name.is_empty() {
                        // Nothing saved: suggest the signed-in user.
                        let user = session.user.get_untracked().unwrap_or_default();
                        name.set(user.display_name);
                        phone.set(String::new());
                        email.set(user.email);
                    } else {
                        name.set(contact.name);
                        phone.set(contact.phone);
                        email.set(contact.email);
                    }
                    months.set(list.into_inner().months);
                    company.set(company_id);
                }
                (Err(status), _) | (_, Err(status)) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's months on screen.
        months.set(Vec::new());
        error.set(None);
        load();
    });
    let changed = Callback::new(move |()| load());

    let save_contact = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        let request = ppb::SetAgiContactRequest {
            company_id: company.get_untracked(),
            contact: Some(ppb::AgiContact {
                name: name.get_untracked(),
                phone: phone.get_untracked(),
                email: email.get_untracked(),
            }),
        };
        spawn_local(async move {
            match payroll_api().set_agi_contact(request).await {
                Ok(_) => load(),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Arbetsgivardeklaration"</h1>
            <ErrorAlert message=error />
            <Card title="Kontaktperson" description="Den som Skatteverket kan kontakta om arbetsgivardeklarationen.">
                <form class="grid grid-cols-3 items-end gap-4" novalidate on:submit=save_contact>
                    <Field label="Namn" id="agi_name" value=name />
                    <Field label="Telefon" id="agi_phone" value=phone kind="tel" />
                    <Field label="E-post" id="agi_email" value=email kind="email" />
                    <div class="col-span-3">
                        <Button disabled=busy>"Spara kontaktperson"</Button>
                    </div>
                </form>
            </Card>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Period"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Ersättning"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatteavdrag"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Arbetsgivaravgifter"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let company_id = company.get();
                            months.get().into_iter().map(|m| (company_id.clone(), m)).collect::<Vec<_>>()
                        }
                        key=|(company_id, m)| (company_id.clone(), m.period.clone(), m.status, m.gross, m.tax_sum, m.fee_sum)
                        let((company_id, summary))
                    >
                        <MonthRow company_id=company_id summary=summary has_contact=has_contact changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn MonthRow(
    company_id: String,
    summary: ppb::AgiMonthSummary,
    has_contact: RwSignal<bool>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    let reference = StoredValue::new(ppb::AgiMonthRef {
        company_id,
        period: summary.period.clone(),
    });
    let submitted = summary.status() == ppb::AgiStatus::Submitted;
    let expanded = RwSignal::new(false);
    let detail = RwSignal::new(None::<ppb::AgiMonth>);
    // The fingerprint of the last file downloaded here; marking sends it, or
    // else that of the month shown, so only what the user saw is marked.
    let downloaded = RwSignal::new(None::<String>);
    let confirming = RwSignal::new(false);
    let label = period_label(&summary.period);

    let toggle = move |_| {
        expanded.update(|e| *e = !*e);
        if expanded.get_untracked() && detail.get_untracked().is_none() {
            let request = reference.get_value();
            spawn_local(async move {
                match payroll_api().get_agi_month(request).await {
                    Ok(month) => detail.set(Some(month.into_inner())),
                    Err(status) => error.set(Some(describe(&status))),
                }
            });
        }
    };
    let download = move |_| {
        error.set(None);
        let request = reference.get_value();
        spawn_local(async move {
            match payroll_api().export_agi_file(request).await {
                Ok(file) => {
                    let file = file.into_inner();
                    save_as(&file.file_name, "application/xml", &file.xml);
                    downloaded.set(Some(file.fingerprint));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let submit = move |_| {
        error.set(None);
        let month = reference.get_value();
        let fingerprint = downloaded
            .get_untracked()
            .or_else(|| detail.get_untracked().map(|m| m.fingerprint))
            .unwrap_or_default();
        let request = ppb::MarkAgiSubmittedRequest {
            company_id: month.company_id,
            period: month.period,
            fingerprint,
        };
        spawn_local(async move {
            match payroll_api().mark_agi_submitted(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>
                <button type="button" aria-expanded=move || expanded.get().to_string() on:click=toggle>
                    {label}
                </button>
            </td>
            <td class=TABLE_AMOUNT_CELL>{kronor(summary.gross)}</td>
            <td class=TABLE_AMOUNT_CELL>{kronor(summary.tax_sum)}</td>
            <td class=TABLE_AMOUNT_CELL>{kronor(summary.fee_sum)}</td>
            <td class=TABLE_CELL>{agi_status_label(summary.status())}</td>
        </tr>
        <Show when=move || expanded.get()>
            <tr class=TABLE_ROW>
                <td class=TABLE_CELL colspan="5">
                    {move || detail.get().map(|month| {
                        let fee_sum = month.summary.as_ref().map_or(0, |s| s.fee_sum);
                        let difference = fee_sum * 100 - month.booked_fees;
                        let booked = format!(
                            "Avgifter enligt bokföringen: {}{}",
                            amount(month.booked_fees),
                            if difference == 0 { String::new() } else { format!(" (skillnad {} från avrundning)", amount(difference.abs())) },
                        );
                        view! {
                            <Table>
                                <thead class=TABLE_HEAD>
                                    <tr class=TABLE_ROW>
                                        <th class=TABLE_HEADER_CELL>"Anställd"</th>
                                        <th class=TABLE_HEADER_CELL>"Personnummer"</th>
                                        <th class=TABLE_HEADER_CELL>"Spec.nr"</th>
                                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Ersättning"</th>
                                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatt"</th>
                                        <th class=TABLE_HEADER_CELL>"Ändring"</th>
                                    </tr>
                                </thead>
                                <tbody class=TABLE_BODY>
                                    {month.lines.into_iter().map(|l| {
                                        let change = agi_change_label(l.change());
                                        view! {
                                            <tr class=TABLE_ROW>
                                                <td class=TABLE_CELL>{l.employee_name}</td>
                                                <td class=format!("{TABLE_CELL} tabular-nums")>{l.personal_identity_number}</td>
                                                <td class=TABLE_CELL>{l.specification_number}</td>
                                                <td class=TABLE_AMOUNT_CELL>{kronor(l.gross)}</td>
                                                <td class=TABLE_AMOUNT_CELL>{kronor(l.tax)}</td>
                                                <td class=TABLE_CELL>{change}</td>
                                            </tr>
                                        }
                                    }).collect_view()}
                                </tbody>
                            </Table>
                            <p class="text-xs/relaxed text-muted-foreground">{booked}</p>
                        }
                    })}
                    <Show when=move || !submitted>
                        <div class="mt-3 flex flex-wrap items-center gap-2">
                            <Button variant=Variant::Ghost kind="button" disabled=Signal::derive(move || !has_contact.get()) on:click=download>
                                "Ladda ner fil"
                            </Button>
                            <Show
                                when=move || confirming.get()
                                fallback=move || view! {
                                    <Button kind="button" disabled=Signal::derive(move || !has_contact.get()) on:click=move |_| confirming.set(true)>
                                        "Markera som inlämnad"
                                    </Button>
                                }
                            >
                                <span class="text-muted-foreground">
                                    "Markera som inlämnad när filen är uppladdad hos Skatteverket."
                                </span>
                                <Button kind="button" on:click=submit>"Bekräfta"</Button>
                            </Show>
                            <Show when=move || !has_contact.get()>
                                <span class="text-muted-foreground">"Spara en kontaktperson först."</span>
                            </Show>
                        </div>
                    </Show>
                </td>
            </tr>
        </Show>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ppb::{AgiChange, AgiStatus};

    #[test]
    fn periods_statuses_and_changes_read_in_swedish() {
        assert_eq!(period_label("202610"), "oktober 2026");
        assert_eq!(period_label("202601"), "januari 2026");
        assert_eq!(period_label("garbage"), "garbage");
        assert_eq!(agi_status_label(AgiStatus::NotSubmitted), "Ej deklarerad");
        assert_eq!(agi_status_label(AgiStatus::Submitted), "Deklarerad");
        assert_eq!(agi_status_label(AgiStatus::Changed), "Ändrad");
        assert_eq!(agi_change_label(AgiChange::New), "Ny");
        assert_eq!(agi_change_label(AgiChange::Changed), "Ändrad");
        assert_eq!(agi_change_label(AgiChange::Removed), "Borttag");
        assert_eq!(agi_change_label(AgiChange::Unchanged), "–");
    }
}
