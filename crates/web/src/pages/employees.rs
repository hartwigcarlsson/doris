//! The active company's employees: add, edit, deactivate. The
//! personnummer is set once and never edited.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::errors::{describe, describe_code};
use crate::format::{amount, parse_amount};
use crate::pages::payroll_runs::tax_setting_label;
use crate::ui::{
    Badge, BadgeVariant, Button, Card, Checkbox, ErrorAlert, Field, PageHeader, SELECT_OPTION,
    Select, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW,
    Table, TableCard, Variant,
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

/// Skatteverket's columns, as offered in the form.
pub const TAX_COLUMNS: [(u32, &str); 6] = [
    (1, "1 – Lön (under 66 år)"),
    (2, "2 – Pension (66 år eller äldre)"),
    (3, "3 – Lön (66 år eller äldre)"),
    (4, "4 – Sjuk- och aktivitetsersättning"),
    (5, "5 – Annan pensionsgrundande ersättning"),
    (6, "6 – Pension (under 66 år)"),
];

/// The error code for a Skatt choice that can't be saved: "table" needs a table.
pub fn tax_fields_error(kind: &str, table: &str) -> Option<&'static str> {
    (kind == "table" && table.trim().is_empty()).then_some("invalid_tax_table")
}

/// The Skatt fields as a setting: "table", "percent" or "none". A field that
/// isn't a number becomes a value the server refuses with its own message.
pub fn tax_input(kind: &str, table: &str, column: &str, percent: &str) -> Option<ppb::TaxSetting> {
    let number = |s: &str, fallback: u32| s.trim().parse().unwrap_or(fallback);
    let kind = match kind {
        "table" => ppb::tax_setting::Kind::Table(ppb::TableTax {
            table: number(table, 0),
            column: number(column, 0),
        }),
        "percent" => ppb::tax_setting::Kind::Percent(number(percent, 1000)),
        _ => return None,
    };
    Some(ppb::TaxSetting { kind: Some(kind) })
}

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
    let tax_kind = RwSignal::new("table".to_owned());
    let tax_table = RwSignal::new(String::new());
    let tax_column = RwSignal::new("1".to_owned());
    let tax_percent = RwSignal::new(String::new());
    // The setting of the employee being edited: it can change, not go away.
    let edited_tax = RwSignal::new(None::<ppb::TaxSetting>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);

    let clear_form = move || {
        editing.set(None);
        name.set(String::new());
        personnummer.set(String::new());
        salary.set(String::new());
        account.set("7210".to_owned());
        tax_kind.set("table".to_owned());
        tax_table.set(String::new());
        tax_column.set("1".to_owned());
        tax_percent.set(String::new());
        edited_tax.set(None);
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
        let (kind, table, column, percent) = match e.tax.as_ref().and_then(|t| t.kind.as_ref()) {
            Some(ppb::tax_setting::Kind::Table(t)) => (
                "table",
                t.table.to_string(),
                t.column.to_string(),
                String::new(),
            ),
            Some(ppb::tax_setting::Kind::Percent(p)) => {
                ("percent", String::new(), "1".to_owned(), p.to_string())
            }
            None => ("none", String::new(), "1".to_owned(), String::new()),
        };
        tax_kind.set(kind.to_owned());
        tax_table.set(table);
        tax_column.set(column);
        tax_percent.set(percent);
        edited_tax.set(e.tax);
    });

    let save = move |ev: SubmitEvent| {
        ev.prevent_default();
        error.set(None);
        if let Some(code) = tax_fields_error(&tax_kind.get_untracked(), &tax_table.get_untracked())
        {
            error.set(Some(describe_code(code)));
            return;
        }
        busy.set(true);
        // The company whose employees are on screen, not whatever is active now.
        let company_id = employees.with_untracked(|(id, _)| id.clone());
        // Not an amount: 0, which the server refuses with its own message.
        let monthly_salary = parse_amount(&salary.get_untracked()).unwrap_or(0);
        let salary_account = account.get_untracked().parse().unwrap_or(0);
        let tax = tax_input(
            &tax_kind.get_untracked(),
            &tax_table.get_untracked(),
            &tax_column.get_untracked(),
            &tax_percent.get_untracked(),
        );
        spawn_local(async move {
            let result = match editing.get_untracked() {
                None => payroll_api()
                    .add_employee(ppb::AddEmployeeRequest {
                        company_id,
                        name: name.get_untracked(),
                        personal_identity_number: personnummer.get_untracked(),
                        monthly_salary,
                        salary_account,
                        tax,
                    })
                    .await
                    .map(|_| ()),
                Some(employee_id) => {
                    let updated = payroll_api()
                        .update_employee(ppb::UpdateEmployeeRequest {
                            company_id: company_id.clone(),
                            employee_id: employee_id.clone(),
                            name: name.get_untracked(),
                            monthly_salary,
                            salary_account,
                        })
                        .await;
                    match updated {
                        Ok(_) if tax.is_some() && tax != edited_tax.get_untracked() => {
                            payroll_api()
                                .set_employee_tax(ppb::SetEmployeeTaxRequest {
                                    company_id,
                                    employee_id,
                                    tax,
                                })
                                .await
                                .map(|_| ())
                        }
                        other => other.map(|_| ()),
                    }
                }
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
        <div class="grid gap-6">
            <PageHeader title="Anställda" />
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
                    <Select label="Skatt" id="tax_kind" value=tax_kind>
                        <option class=SELECT_OPTION value="table">"Skattetabell"</option>
                        <option class=SELECT_OPTION value="percent">"Fast procent"</option>
                        <option
                            class=SELECT_OPTION
                            value="none"
                            disabled=move || edited_tax.get().is_some()
                            hidden=move || edited_tax.get().is_some()
                        >
                            "Ingen (skatten skrivs in för hand)"
                        </option>
                    </Select>
                    <Show when=move || tax_kind.get() == "table">
                        <Select label="Tabell" id="tax_table" value=tax_table>
                            <option class=SELECT_OPTION value="">"Välj…"</option>
                            {(29..=42u32)
                                .map(|t| view! { <option class=SELECT_OPTION value=t.to_string()>{t}</option> })
                                .collect_view()}
                        </Select>
                        <Select label="Kolumn" id="tax_column" value=tax_column>
                            {TAX_COLUMNS
                                .map(|(n, label)| view! { <option class=SELECT_OPTION value=n.to_string()>{label}</option> })
                                .collect_view()}
                        </Select>
                    </Show>
                    <Show when=move || tax_kind.get() == "percent">
                        <Field label="Procent" id="tax_percent" value=tax_percent />
                    </Show>
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
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Namn"</th>
                        <th class=TABLE_HEADER_CELL>"Personnummer"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Månadslön"</th>
                        <th class=TABLE_HEADER_CELL>"Konto"</th>
                        <th class=TABLE_HEADER_CELL>"Skatt"</th>
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
                            (company_id.clone(), e.id.clone(), e.name.clone(), e.monthly_salary, e.salary_account, e.active, tax_setting_label(e.tax.as_ref()))
                        }
                        let((company_id, employee))
                    >
                        <EmployeeRow company_id=company_id employee=employee edit=edit changed=changed error=error />
                    </For>
                </tbody>
            </Table></TableCard>
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
            <td class=TABLE_CELL>{tax_setting_label(employee.tax.as_ref())}</td>
            <td class=TABLE_CELL>
                {if employee.active {
                    view! { <Badge>"Aktiv"</Badge> }.into_any()
                } else {
                    view! { <Badge variant=BadgeVariant::Outline>"Inaktiv"</Badge> }.into_any()
                }}
            </td>
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

#[cfg(test)]
mod tests {
    use super::{tax_fields_error, tax_input};
    use crate::api::ppb::{TableTax, tax_setting::Kind};

    #[test]
    fn a_table_setting_without_a_table_is_caught_before_saving() {
        assert_eq!(tax_fields_error("table", " "), Some("invalid_tax_table"));
        assert_eq!(tax_fields_error("table", "33"), None);
        assert_eq!(tax_fields_error("percent", ""), None);
        assert_eq!(tax_fields_error("none", ""), None);
    }

    #[test]
    fn the_tax_fields_become_a_setting_or_none() {
        assert_eq!(
            tax_input("table", "33", "1", "").and_then(|t| t.kind),
            Some(Kind::Table(TableTax {
                table: 33,
                column: 1
            }))
        );
        assert_eq!(
            tax_input("percent", "", "", " 30 ").and_then(|t| t.kind),
            Some(Kind::Percent(30))
        );
        assert_eq!(tax_input("none", "33", "1", "30"), None);
        // Not a number: 0, which the server refuses with its own message.
        assert_eq!(
            tax_input("table", "", "1", "").and_then(|t| t.kind),
            Some(Kind::Table(TableTax {
                table: 0,
                column: 1
            }))
        );
        assert_eq!(
            tax_input("percent", "", "", "tre").and_then(|t| t.kind),
            Some(Kind::Percent(1000))
        );
    }
}
