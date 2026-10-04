//! The active company's employees: add, edit, deactivate. The
//! personnummer is set once and never edited.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, parse_amount};
use crate::ui::{
    Button, Card, Checkbox, ErrorAlert, Field, SELECT_OPTION, Select, TABLE_AMOUNT_CELL,
    TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, Variant,
};
use leptos::ev::SubmitEvent;
use leptos::prelude::*;
use leptos::task::spawn_local;

/// The salary accounts an employee can have, in the order offered.
pub const SALARY_ACCOUNTS: [(u32, &str); 3] = [
    (7210, "7210 Löner till tjänstemän"),
    (7010, "7010 Löner till kollektivanställda"),
    (7220, "7220 Löner till företagsledare"),
];

#[component]
pub fn Employees() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // The employees and the company they were loaded for, set together.
    let employees = RwSignal::new((String::new(), Vec::<ppb::Employee>::new()));
    let show_inactive = RwSignal::new(false);
    // The employee being edited; `None` while adding.
    let editing = RwSignal::new(None::<String>);
    let name = RwSignal::new(String::new());
    let personnummer = RwSignal::new(String::new());
    let salary = RwSignal::new(String::new());
    let account = RwSignal::new("7210".to_owned());
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let clear_form = move || {
        editing.set(None);
        name.set(String::new());
        personnummer.set(String::new());
        salary.set(String::new());
        account.set("7210".to_owned());
    };
    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = payroll_api()
                .list_employees(ppb::ListEmployeesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => employees.set((company_id, response.into_inner().employees)),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        // Never leave the previous company's rows (or forms) on screen.
        employees.set((String::new(), Vec::new()));
        error.set(None);
        clear_form();
        load();
    });
    let changed = Callback::new(move |()| load());
    let edit = Callback::new(move |e: ppb::Employee| {
        error.set(None);
        editing.set(Some(e.id));
        name.set(e.name);
        personnummer.set(e.personal_identity_number);
        salary.set(amount(e.monthly_salary));
        account.set(e.salary_account.to_string());
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        busy.set(true);
        error.set(None);
        // The company whose employees are on screen, not whatever is active now.
        let company_id = employees.with_untracked(|(id, _)| id.clone());
        // Not an amount: 0, which the server refuses with its own message.
        let monthly_salary = parse_amount(&salary.get_untracked()).unwrap_or(0);
        let salary_account = account.get_untracked().parse().unwrap_or(0);
        spawn_local(async move {
            let result = match editing.get_untracked() {
                None => payroll_api()
                    .add_employee(ppb::AddEmployeeRequest {
                        company_id,
                        name: name.get_untracked(),
                        personal_identity_number: personnummer.get_untracked(),
                        monthly_salary,
                        salary_account,
                    })
                    .await
                    .map(|_| ()),
                Some(employee_id) => payroll_api()
                    .update_employee(ppb::UpdateEmployeeRequest {
                        company_id,
                        employee_id,
                        name: name.get_untracked(),
                        monthly_salary,
                        salary_account,
                    })
                    .await
                    .map(|_| ()),
            };
            match result {
                Ok(()) => {
                    clear_form();
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">"Anställda"</h1>
            <ErrorAlert message=error />
            <Card title="Anställd">
                <form class="grid grid-cols-2 items-end gap-4" novalidate on:submit=save>
                    <Field label="Namn" id="employee_name" value=name />
                    <Show when=move || editing.get().is_none()>
                        <Field
                            label="Personnummer"
                            id="personal_identity_number"
                            value=personnummer
                            placeholder="ÅÅÅÅMMDD-NNNN"
                        />
                    </Show>
                    <Field label="Månadslön (kr)" id="monthly_salary" value=salary />
                    <Select label="Lönekonto" id="salary_account" value=account>
                        {SALARY_ACCOUNTS
                            .map(|(number, label)| {
                                view! { <option class=SELECT_OPTION value=number.to_string()>{label}</option> }
                            })
                            .collect_view()}
                    </Select>
                    <div class="col-span-2 flex gap-2">
                        <Button disabled=busy>
                            {move || if editing.get().is_some() { "Spara ändringar" } else { "Lägg till anställd" }}
                        </Button>
                        <Show when=move || editing.get().is_some()>
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| clear_form()>"Avbryt"</Button>
                        </Show>
                    </div>
                </form>
            </Card>
            <Checkbox label="Visa inaktiva" id="show_inactive" checked=show_inactive />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Personnummer"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Månadslön"</th>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL></th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For
                        each=move || {
                            let (company_id, list) = employees.get();
                            list.into_iter()
                                .filter(|e| e.active || show_inactive.get())
                                .map(|e| (company_id.clone(), e))
                                .collect::<Vec<_>>()
                        }
                        key=|(company_id, e)| {
                            (company_id.clone(), e.id.clone(), e.name.clone(), e.monthly_salary, e.salary_account, e.active)
                        }
                        let((company_id, employee))
                    >
                        <EmployeeRow company_id=company_id employee=employee edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

#[component]
fn EmployeeRow(
    company_id: String,
    employee: ppb::Employee,
    edit: Callback<ppb::Employee>,
    changed: Callback<()>,
    error: RwSignal<Option<String>>,
) -> impl IntoView {
    // The company this row was loaded for, not whatever is active now.
    let company_id = StoredValue::new(company_id);
    let stored = StoredValue::new(employee.clone());
    let confirming = RwSignal::new(false);
    let deactivate = move |_| {
        error.set(None);
        spawn_local(async move {
            let request = ppb::DeactivateEmployeeRequest {
                company_id: company_id.get_value(),
                employee_id: stored.with_value(|e| e.id.clone()),
            };
            match payroll_api().deactivate_employee(request).await {
                Ok(_) => changed.run(()),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let account = SALARY_ACCOUNTS
        .iter()
        .find(|(number, _)| *number == employee.salary_account)
        .map_or_else(
            || employee.salary_account.to_string(),
            |(_, label)| (*label).to_owned(),
        );

    view! {
        <tr class=TABLE_ROW>
            <td class=TABLE_CELL>{employee.name.clone()}</td>
            <td class=format!("{TABLE_CELL} tabular-nums")>{employee.personal_identity_number.clone()}</td>
            <td class=TABLE_AMOUNT_CELL>{amount(employee.monthly_salary)}</td>
            <td class=TABLE_CELL>{account}</td>
            <td class=TABLE_CELL>{if employee.active { "Aktiv" } else { "Inaktiv" }}</td>
            <td class=format!("{TABLE_CELL} text-right")>
                <Show when=move || stored.with_value(|e| e.active)>
                    <Button variant=Variant::Ghost kind="button" on:click=move |_| edit.run(stored.get_value())>
                        "Redigera"
                    </Button>
                    <Show
                        when=move || confirming.get()
                        fallback=move || view! {
                            <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>
                                "Inaktivera"
                            </Button>
                        }
                    >
                        <Button kind="button" on:click=deactivate>"Bekräfta inaktivering"</Button>
                    </Show>
                </Show>
            </td>
        </tr>
    }
}
