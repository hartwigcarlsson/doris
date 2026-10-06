//! The active company's chart of accounts: add, rename, (de)activate.

use crate::active_company::Companies;
use crate::api::{ledger_api, lpb};
use crate::errors::describe;
use crate::task::spawn_local;
use crate::ui::{
    Badge, BadgeVariant, Button, Card, Checkbox, ErrorAlert, Field, Icon, IconName, PageHeader,
    SELECT, SELECT_OPTION, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW,
    Table, TableCard, TextInput, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;

/// "05 Momspliktig försäljning…", or "–" for no box.
pub fn vat_box_text(n: u32) -> String {
    match crate::vat_form::box_label(n) {
        Some(label) if n != 49 => format!("{n:02} {label}"),
        _ => "–".into(),
    }
}

#[component]
pub fn Accounts() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The accounts and the company they were loaded for, set together.
    let accounts = RwSignal::new((String::new(), Vec::<lpb::Account>::new()));
    let show_inactive = RwSignal::new(false);
    let number = RwSignal::new(String::new());
    let name = RwSignal::new(String::new());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let load = move || {
        let company_id = companies.active.get_untracked();
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
            match result {
                Ok(response) => accounts.set((company_id, response.into_inner().accounts)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or forms) on screen.
        accounts.set((String::new(), Vec::new()));
        error.set(None);
        number.set(String::new());
        name.set(String::new());
        load();
    });
    let changed = Callback::new(move |()| load());

    let add = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose chart is on screen, not whatever is active now.
        let company_id = accounts.with_untracked(|(id, _)| id.clone());
        spawn_local(async move {
            let request = lpb::AddAccountRequest {
                company_id,
                // Not a number: 0, which the server refuses with its own message.
                number: number.get_untracked().trim().parse().unwrap_or(0),
                name: name.get_untracked(),
            };
            match ledger_api().add_account(request).await {
                Ok(_) => {
                    number.set(String::new());
                    name.set(String::new());
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6">
            <PageHeader title="Kontoplan" />
            <ErrorAlert message=error />
            <Card title="Lägg till konto">
                <form class="grid grid-cols-[8rem_1fr_auto] items-end gap-4" novalidate on:submit=add>
                    <Field label="Nummer" id="account_number" value=number />
                    <Field label="Namn" id="account_name" value=name />
                    <Button disabled=busy>"Lägg till konto"</Button>
                </form>
            </Card>
            <Checkbox label="Visa inaktiva" id="show_inactive" checked=show_inactive />
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Momsruta"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = accounts.get();
                            list.into_iter()
                                .filter(|a| a.active || show_inactive.get())
                                .map(|a| (company_id.clone(), a))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, a)| (company_id.clone(), a.number, a.name.clone(), a.active, a.vat_box)
                        let((company_id, account))
                    >
                        <AccountRow company_id=company_id account=account changed=changed error=error />
                    </For>
                </tbody>
            </Table></TableCard>
        </div>
    }
}

#[component]
fn AccountRow(
    company_id: String,
    account: lpb::Account,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let lpb::Account {
        number,
        name: current,
        active,
        vat_box,
    } = account;
    let chosen = RwSignal::new(vat_box.to_string());
    Effect::new(move |previous: Option<String>| {
        let value = chosen.get();
        // Only a change by the user, not the first run.
        if previous.is_some_and(|p| p != value) {
            error.set(None);
            let vat_box = value.parse().unwrap_or(0);
            spawn_local(async move {
                let request = lpb::SetAccountVatBoxRequest {
                    company_id: company_id.get_value(),
                    number,
                    vat_box,
                };
                match ledger_api().set_account_vat_box(request).await {
                    Ok(_) => changed.run(()),
                    Err(status) => error.set(Some(describe(&status))),
                }
            });
        }
        value
    });
    let editing = RwSignal::new(false);
    let name = RwSignal::new(current.clone());

    let rename = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        spawn_local(async move {
            let request = lpb::RenameAccountRequest {
                company_id: company_id.get_value(),
                number,
                name: name.get_untracked(),
            };
            match ledger_api().rename_account(request).await {
                Ok(_) => {
                    editing.set(false);
                    changed.run(());
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let toggle = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = lpb::SetAccountActiveRequest {
                company_id: company_id.get_value(),
                number,
                active: !active,
            };
            match ledger_api().set_account_active(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{number}</td>
            <td class=TABLE_CELL>
                <Show
                    when=move || editing.get()
                    fallback={
                        let current = current.clone();
                        move || current.clone()
                    }
                >
                    <form class="flex gap-2" novalidate on:submit=rename>
                        <TextInput label=format!("Nytt namn för {number}") value=name />
                        <Button>"Spara"</Button>
                    </form>
                </Show>
            </td>
            <td class=TABLE_CELL>
                <div class="relative w-64 max-w-full">
                    <select
                        class=SELECT
                        aria-label=format!("Momsruta för {number}")
                        prop:value=move || chosen.get()
                        on:change=move |ev| chosen.set(event_target_value(&ev))
                    >
                        <option class=SELECT_OPTION value="0">"–"</option>
                        {crate::vat_form::SECTIONS.iter().flat_map(|s| s.rows).filter(|r| r.vat_box != 49).map(|r| view! {
                            <option class=SELECT_OPTION value=r.vat_box.to_string()>{vat_box_text(r.vat_box)}</option>
                        }).collect_view()}
                    </select>
                    <Icon name=IconName::ChevronDown class="pointer-events-none absolute top-1/2 right-1.5 size-3.5 -translate-y-1/2 text-muted-foreground select-none" />
                </div>
            </td>
            <td class=TABLE_CELL>
                {if active {
                    view! { <Badge>"Aktivt"</Badge> }.into_any()
                } else {
                    view! { <Badge variant=BadgeVariant::Outline>"Inaktivt"</Badge> }.into_any()
                }}
            </td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Button variant=Variant::Ghost kind="button" on:click=move |_| editing.set(true)>
                    "Byt namn"
                </Button>
                <Button variant=Variant::Ghost kind="button" on:click=toggle>
                    {if active { "Inaktivera" } else { "Aktivera" }}
                </Button>
            </td>
        </tr>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_box_reads_as_its_number_and_the_forms_text() {
        assert_eq!(vat_box_text(5), "05 Momspliktig försäljning som inte ingår i ruta 06, 07 eller 08");
        assert_eq!(vat_box_text(48), "48 Ingående moms att dra av");
        assert_eq!(vat_box_text(0), "–");
    }
}
