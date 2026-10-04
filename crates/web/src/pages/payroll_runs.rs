//! The active company's payroll runs, newest pay date first.

use crate::active_company::Companies;
use crate::api::{lpb, payroll_api, ppb};
use crate::errors::describe;
use crate::format::{amount, today};
use crate::ui::{
    ErrorAlert, TABLE_AMOUNT_CELL, TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL,
    TABLE_ROW, Table,
};
use leptos::prelude::*;
use leptos::task::spawn_local;
use leptos_router::components::A;

/// The status as the user sees it on `today` (both `YYYY-MM-DD`).
pub fn status_label(status: ppb::PayrollRunStatus, pay_date: &str, today: &str) -> &'static str {
    match status {
        ppb::PayrollRunStatus::Open => "Öppen",
        ppb::PayrollRunStatus::Finalized if pay_date <= today => "Att bokföra",
        ppb::PayrollRunStatus::Finalized => "Färdigställd",
        ppb::PayrollRunStatus::Booked => "Bokförd",
        ppb::PayrollRunStatus::Unspecified => "",
    }
}

/// Basis points as a Swedish percentage: 3142 → "31,42 %".
pub fn fee_rate(basis_points: u32) -> String {
    format!("{},{:02} %", basis_points / 100, basis_points % 100)
}

#[component]
pub fn PayrollRuns() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let runs = RwSignal::new(Vec::<ppb::PayrollRun>::new());
    let error = RwSignal::new(None::<String>);
    Effect::new(move |_| {
        let company_id = companies.active.get();
        // Never leave the previous company's runs on screen.
        runs.set(Vec::new());
        error.set(None);
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = payroll_api()
                .list_payroll_runs(ppb::ListPayrollRunsRequest {
                    company_id: company_id.clone(),
                })
                .await;
            // The user switched company meanwhile: this answer is stale.
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(response) => runs.set(response.into_inner().payroll_runs),
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    });
    let today = today();

    view! {
        <div class="grid gap-6" data-wide>
            <div class="flex items-end justify-between gap-4">
                <h1 class="text-sm font-medium">"Lönekörningar"</h1>
                <A href="/payroll-runs/new" attr:class="text-xs/relaxed font-medium underline-offset-4 hover:underline">"Ny lönekörning"</A>
            </div>
            <ErrorAlert message=error />
            <Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Utbetalningsdag"</th>
                        <th class=TABLE_HEADER_CELL>"Text"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Brutto"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatt"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgift"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Netto"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL>"Ver"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    <For each=move || runs.get() key=|run| (run.id.clone(), run.status, run.text.clone(), run.pay_date.clone()) let(run)>
                        {
                            let sum = |amount: fn(&ppb::PayrollRunLine) -> i64| run.lines.iter().map(amount).sum::<i64>();
                            let locked = run.status() != ppb::PayrollRunStatus::Open;
                            let (gross, tax, fee, net) = (sum(|l| l.gross), sum(|l| l.tax), sum(|l| l.fee), sum(|l| l.net));
                            let label = status_label(run.status(), &run.pay_date, &today);
                            let shown = move |ore: i64| if locked { amount(ore) } else { "–".to_owned() };
                            view! {
                                <tr class=TABLE_ROW>
                                    <td class=TABLE_CELL>
                                        <A href=format!("/payroll-runs/{}", run.id)>{run.pay_date.clone()}</A>
                                    </td>
                                    <td class=TABLE_CELL>{run.text.clone()}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(gross)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{amount(tax)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{shown(fee)}</td>
                                    <td class=TABLE_AMOUNT_CELL>{shown(net)}</td>
                                    <td class=TABLE_CELL>{label}</td>
                                    <td class=TABLE_CELL>{run.voucher.as_ref().map(|v| v.number.to_string())}</td>
                                </tr>
                            }
                        }
                    </For>
                </tbody>
            </Table>
        </div>
    }
}

/// A run's lines (with fee and net once locked) and the voucher it books.
#[component]
pub fn RunLines(
    lines: Vec<ppb::PayrollRunLine>,
    voucher_lines: Vec<lpb::VoucherLine>,
) -> impl IntoView {
    view! {
        <Table>
            <thead class=TABLE_HEAD>
                <tr class=TABLE_ROW>
                    <th class=TABLE_HEADER_CELL>"Anställd"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Brutto"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Skatt"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgiftssats"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Avgift"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Netto"</th>
                </tr>
            </thead>
            <tbody class=TABLE_BODY>
                {lines
                    .into_iter()
                    .map(|l| view! {
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>{l.employee_name}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.gross)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.tax)}</td>
                            <td class=TABLE_AMOUNT_CELL>{fee_rate(l.fee_rate)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.fee)}</td>
                            <td class=TABLE_AMOUNT_CELL>{amount(l.net)}</td>
                        </tr>
                    })
                    .collect_view()}
            </tbody>
        </Table>
        <h2 class="text-xs/relaxed font-medium">"Verifikation"</h2>
        <Table>
            <thead class=TABLE_HEAD>
                <tr class=TABLE_ROW>
                    <th class=TABLE_HEADER_CELL>"Konto"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Debet"</th>
                    <th class=format!("{TABLE_HEADER_CELL} text-right")>"Kredit"</th>
                </tr>
            </thead>
            <tbody class=TABLE_BODY>
                {voucher_lines
                    .into_iter()
                    .map(|l| view! {
                        <tr class=TABLE_ROW>
                            <td class=TABLE_CELL>{l.account}</td>
                            <td class=TABLE_AMOUNT_CELL>{(l.debit != 0).then(|| amount(l.debit))}</td>
                            <td class=TABLE_AMOUNT_CELL>{(l.credit != 0).then(|| amount(l.credit))}</td>
                        </tr>
                    })
                    .collect_view()}
            </tbody>
        </Table>
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::ppb::PayrollRunStatus::*;

    #[test]
    fn a_finalized_run_is_to_be_booked_from_its_pay_date() {
        assert_eq!(status_label(Open, "2026-10-25", "2026-10-04"), "Öppen");
        assert_eq!(
            status_label(Finalized, "2026-10-25", "2026-10-24"),
            "Färdigställd"
        );
        assert_eq!(
            status_label(Finalized, "2026-10-25", "2026-10-25"),
            "Att bokföra"
        );
        assert_eq!(status_label(Booked, "2026-10-25", "2026-10-26"), "Bokförd");
    }

    #[test]
    fn fee_rates_are_shown_as_percent() {
        assert_eq!(fee_rate(3142), "31,42 %");
        assert_eq!(fee_rate(1021), "10,21 %");
        assert_eq!(fee_rate(0), "0,00 %");
    }
}
