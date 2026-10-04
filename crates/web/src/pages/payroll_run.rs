//! One payroll run. Öppen: a form (preview, save, finalize). Färdigställd:
//! read-only, with Öppna and Bokför (from the pay date). Bokförd:
//! read-only, with Backa bokföring.

use crate::active_company::Companies;
use crate::api::{payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, parse_amount, today};
use crate::pages::payroll_runs::{RunLines, status_label};
use crate::ui::{
    Button, Checkbox, ErrorAlert, Field, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table, TextInput, Variant,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;
use leptos_router::hooks::{use_navigate, use_params_map};

/// One RPC of the read-only view (Öppna, Bokför, Bekräfta backning).
type Call = std::pin::Pin<Box<dyn std::future::Future<Output = Result<(), tonic::Status>>>>;

/// One employee's row in the form.
#[derive(Clone, Copy)]
struct Row {
    employee_id: StoredValue<String>,
    name: StoredValue<String>,
    included: RwSignal<bool>,
    gross: RwSignal<String>,
    tax: RwSignal<String>,
}

/// Active employees, plus those already in `run` (who may have been
/// deactivated since). A new run includes everyone at their monthly salary.
fn form_rows(employees: &[ppb::Employee], run: Option<&ppb::PayrollRun>) -> Vec<Row> {
    employees
        .iter()
        .filter_map(|e| {
            let line = run.and_then(|r| r.lines.iter().find(|l| l.employee_id == e.id));
            if !e.active && line.is_none() {
                return None;
            }
            Some(Row {
                employee_id: StoredValue::new(e.id.clone()),
                name: StoredValue::new(e.name.clone()),
                included: RwSignal::new(run.is_none() || line.is_some()),
                gross: RwSignal::new(amount(line.map_or(e.monthly_salary, |l| l.gross))),
                tax: RwSignal::new(line.map(|l| amount(l.tax)).unwrap_or_default()),
            })
        })
        .collect()
}

/// Whether `active` replaces another real company. The first company
/// arrives after `previous` was empty (companies load asynchronously), which
/// is a load, not a switch.
fn switched(previous: Option<&str>, active: &str) -> bool {
    previous.is_some_and(|p| !p.is_empty() && p != active)
}

/// The first included employee whose Skatt is blank, as the Swedish message.
/// Blank is not 0: that would silently under-withhold.
fn missing_tax(rows: &[(String, bool, String)]) -> Option<String> {
    rows.iter()
        .find(|(_, included, tax)| *included && tax.trim().is_empty())
        .map(|(name, _, _)| format!("Ange skatt för {name}."))
}

#[component]
pub fn PayrollRunPage() -> impl IntoView {
    let companies = expect_context::<Companies>();
    // None on /payroll-runs/new. Another run is another mount of the page.
    let run_id = use_params_map().read_untracked().get("id");
    let navigate = StoredValue::new_local(use_navigate());
    let go = move |path: String| navigate.with_value(|nav| nav(&path, Default::default()));
    // The run as last loaded, and the company it was loaded for.
    let run = RwSignal::new(None::<ppb::PayrollRun>);
    let company = RwSignal::new(String::new());
    let rows = RwSignal::new(Vec::<Row>::new());
    let pay_date = RwSignal::new(String::new());
    let text = RwSignal::new(String::new());
    let preview = RwSignal::new(None::<ppb::PreviewPayrollRunResponse>);
    let error = RwSignal::new(None::<String>);
    let busy = RwSignal::new(false);
    let confirming = RwSignal::new(false);
    let run_id = StoredValue::new(run_id);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        let id = run_id.get_value();
        spawn_local(async move {
            let employees = payroll_api()
                .list_employees(ppb::ListEmployeesRequest {
                    company_id: company_id.clone(),
                })
                .await;
            let loaded = match id {
                Some(payroll_run_id) => Some(
                    payroll_api()
                        .get_payroll_run(ppb::PayrollRunRef {
                            company_id: company_id.clone(),
                            payroll_run_id,
                        })
                        .await,
                ),
                None => None,
            };
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            let employees = match employees {
                Ok(response) => response.into_inner().employees,
                Err(status) => return error.set(Some(describe(&status))),
            };
            let loaded = match loaded {
                Some(Ok(response)) => Some(response.into_inner()),
                Some(Err(status)) => return error.set(Some(describe(&status))),
                None => None,
            };
            rows.set(form_rows(&employees, loaded.as_ref()));
            pay_date.set(loaded.as_ref().map_or_else(today, |r| r.pay_date.clone()));
            text.set(loaded.as_ref().map(|r| r.text.clone()).unwrap_or_default());
            preview.set(None);
            confirming.set(false);
            company.set(company_id);
            run.set(loaded);
        });
    };
    Effect::new(move |previous: Option<String>| {
        let active = companies.active.get();
        // A run belongs to one company: on a switch, back to the list.
        if switched(previous.as_deref(), &active) {
            go("/payroll-runs".to_owned());
        } else {
            load();
        }
        active
    });

    let draft = move || ppb::PayrollRunDraft {
        pay_date: pay_date.get_untracked(),
        text: text.get_untracked(),
        lines: rows
            .get_untracked()
            .iter()
            .filter(|r| r.included.get_untracked())
            .map(|r| ppb::PayrollRunLineInput {
                employee_id: r.employee_id.get_value(),
                // Not an amount: 0 or -1, which the server refuses with its own message.
                gross: parse_amount(&r.gross.get_untracked()).unwrap_or(0),
                tax: parse_amount(&r.tax.get_untracked()).unwrap_or(-1),
            })
            .collect(),
    };
    // Refuses a draft with a blank Skatt before any RPC; shows why.
    let tax_missing = move || {
        let form: Vec<_> = rows
            .get_untracked()
            .iter()
            .map(|r| {
                (
                    r.name.get_value(),
                    r.included.get_untracked(),
                    r.tax.get_untracked(),
                )
            })
            .collect();
        let message = missing_tax(&form);
        let missing = message.is_some();
        if missing {
            error.set(message);
        }
        missing
    };
    // The preview shows the form as it was: any edit clears it.
    Effect::new(move |_| {
        pay_date.track();
        text.track();
        for r in rows.get() {
            r.included.track();
            r.gross.track();
            r.tax.track();
        }
        preview.set(None);
    });
    // The run on screen, for the company it was loaded for (or, for a new
    // run, the active one).
    let reference = move || ppb::PayrollRunRef {
        company_id: company.get_untracked(),
        payroll_run_id: run
            .with_untracked(|r| r.as_ref().map(|r| r.id.clone()).unwrap_or_default()),
    };
    // Runs `call` and reloads, showing its error.
    let act = move |call: Call| {
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            match call.await {
                Ok(()) => load(),
                Err(status) => error.set(Some(describe(&status))),
            }
            busy.set(false);
        });
    };

    let preview_click = move |_| {
        if tax_missing() {
            return;
        }
        let request = ppb::PreviewPayrollRunRequest {
            company_id: company.get_untracked(),
            draft: Some(draft()),
        };
        error.set(None);
        spawn_local(async move {
            match payroll_api().preview_payroll_run(request).await {
                Ok(response) => preview.set(Some(response.into_inner())),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    // Saves the form (creating the run if new), then finalizes if asked.
    let save = move |finalize: bool| {
        if tax_missing() {
            return;
        }
        let (company_id, existing, draft) = (
            company.get_untracked(),
            run.with_untracked(|r| r.as_ref().map(|r| r.id.clone())),
            draft(),
        );
        busy.set(true);
        error.set(None);
        spawn_local(async move {
            let saved = match existing.clone() {
                None => payroll_api()
                    .create_payroll_run(ppb::CreatePayrollRunRequest {
                        company_id: company_id.clone(),
                        draft: Some(draft),
                    })
                    .await
                    .map(|r| r.into_inner().payroll_run_id),
                Some(payroll_run_id) => payroll_api()
                    .update_payroll_run(ppb::UpdatePayrollRunRequest {
                        run: Some(ppb::PayrollRunRef {
                            company_id: company_id.clone(),
                            payroll_run_id: payroll_run_id.clone(),
                        }),
                        draft: Some(draft),
                    })
                    .await
                    .map(|_| payroll_run_id),
            };
            let id = match saved {
                Ok(id) => id,
                Err(status) => {
                    busy.set(false);
                    return error.set(Some(describe(&status)));
                }
            };
            let result = if finalize {
                payroll_api()
                    .finalize_payroll_run(ppb::PayrollRunRef {
                        company_id,
                        payroll_run_id: id.clone(),
                    })
                    .await
                    .map(|_| ())
            } else {
                Ok(())
            };
            busy.set(false);
            // A created run is open on its own page even if finalizing failed:
            // staying on /new would let a second press create a duplicate.
            if existing.is_none() {
                go(format!("/payroll-runs/{id}"));
            } else {
                match result {
                    Ok(()) => load(),
                    Err(status) => error.set(Some(describe(&status))),
                }
            }
        });
    };

    let form = move || {
        view! {
            <div class="grid grid-cols-2 gap-4">
                <Field label="Utbetalningsdag" id="pay_date" value=pay_date kind="date" />
                <Field label="Text" id="payroll_run_text" value=text placeholder="Lön {månad år}" />
            </div>
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Anställd"</th>
                        <th class=TABLE_HEADER_CELL>"Brutto (kr)"</th>
                        <th class=TABLE_HEADER_CELL>"Skatt (kr)"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || rows.get() key=|row| row.employee_id.get_value() let(row)>
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>
                                <Checkbox
                                    label=row.name.get_value()
                                    id=format!("include-{}", row.employee_id.get_value())
                                    checked=row.included
                                />
                            </td>
                            <td class=TABLE_CELL>
                                <TextInput label=format!("Brutto, {}", row.name.get_value()) value=row.gross inputmode="decimal" />
                            </td>
                            <td class=TABLE_CELL>
                                <TextInput label=format!("Skatt, {}", row.name.get_value()) value=row.tax inputmode="decimal" />
                            </td>
                        </tr>
                    </For>
                </tbody>
            </Table>
            <div class="flex gap-2">
                <Button variant=Variant::Ghost kind="button" disabled=busy on:click=preview_click>"Förhandsgranska"</Button>
                <Button variant=Variant::Ghost kind="button" disabled=busy on:click=move |_| save(false)>"Spara"</Button>
                <Button kind="button" disabled=busy on:click=move |_| save(true)>"Färdigställ"</Button>
            </div>
            {move || preview.get().map(|p| view! { <RunLines lines=p.lines voucher_lines=p.voucher_lines /> })}
        }
    };

    let locked = move |r: ppb::PayrollRun| {
        let due = r.pay_date <= today();
        let status = r.status();
        let booked = r.voucher.clone();
        view! {
            <p class="text-xs/relaxed">
                "Utbetalningsdag " {r.pay_date.clone()} " · " {status_label(status, &r.pay_date, &today())}
            </p>
            <RunLines lines=r.lines.clone() voucher_lines=r.voucher_lines.clone() />
            {match booked {
                None => view! {
                    <div class="flex items-center gap-2">
                        <Button variant=Variant::Ghost kind="button" disabled=busy on:click=move |_| {
                            let r = reference();
                            act(Box::pin(async move { payroll_api().reopen_payroll_run(r).await.map(|_| ()) }))
                        }>"Öppna"</Button>
                        <Button kind="button" disabled=Signal::derive(move || busy.get() || !due) on:click=move |_| {
                            let r = reference();
                            act(Box::pin(async move { payroll_api().book_payroll_run(r).await.map(|_| ()) }))
                        }>"Bokför"</Button>
                        {(!due).then(|| view! {
                            <span class="text-muted-foreground">{format!("Kan bokföras från {}", r.pay_date)}</span>
                        })}
                    </div>
                }
                .into_any(),
                Some(voucher) => view! {
                    <div class="flex items-center gap-2">
                        <A href="/vouchers">{format!("Ver {}", voucher.number)}</A>
                        <Show
                            when=move || confirming.get()
                            fallback=move || view! {
                                <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(true)>
                                    "Backa bokföring"
                                </Button>
                            }
                        >
                            <span class="text-muted-foreground">
                                "En rättelse bokförs med dagens datum. Körningen blir färdigställd igen."
                            </span>
                            <Button kind="button" disabled=busy on:click=move |_| {
                                let r = reference();
                                act(Box::pin(async move { payroll_api().unbook_payroll_run(r).await.map(|_| ()) }))
                            }>"Bekräfta backning"</Button>
                        </Show>
                    </div>
                }
                .into_any(),
            }}
        }
    };

    view! {
        <div class="grid gap-6" data-wide>
            <h1 class="text-sm font-medium">
                {move || run.with(|r| r.as_ref().map_or_else(|| "Ny lönekörning".to_owned(), |r| r.text.clone()))}
            </h1>
            <ErrorAlert message=error />
            {move || match run.get() {
                Some(r) if r.status() != ppb::PayrollRunStatus::Open => locked(r).into_any(),
                _ => form().into_any(),
            }}
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::{missing_tax, switched};

    #[test]
    fn blank_tax_of_an_included_employee_is_reported_by_name() {
        let row = |n: &str, included, tax: &str| (n.to_owned(), included, tax.to_owned());
        let rows = [
            row("Ann", false, ""),
            row("Bo", true, "  "),
            row("Cy", true, ""),
        ];
        assert_eq!(missing_tax(&rows).as_deref(), Some("Ange skatt för Bo."));
        assert_eq!(
            missing_tax(&[row("Ann", true, "0"), row("Bo", true, "x")]),
            None
        );
    }

    #[test]
    fn the_first_company_arriving_is_not_a_switch() {
        assert!(!switched(None, "a"));
        assert!(!switched(Some(""), "a"));
        assert!(!switched(Some("a"), "a"));
        assert!(switched(Some("a"), "b"));
    }
}
