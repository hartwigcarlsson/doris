//! Moms: the chosen räkenskapsår's redovisningsperiod and its periods,
//! each with status, deklarationsdag and box 49.

use crate::active_company::Companies;
use crate::api::{vat_api, vpb};
use crate::errors::describe;
use crate::fiscal_year::{FiscalYearSelect, keep_year_in_url, use_fiscal_years};
use crate::format::amount;
use crate::overview::whole_kronor;
use crate::task::spawn_local;
use crate::ui::{
    Badge, BadgeVariant, Card, ErrorAlert, PageHeader, SELECT_OPTION, Select, TABLE_AMOUNT_CELL,
    TABLE_BODY, TABLE_CELL, TABLE_HEAD, TABLE_HEADER_CELL, TABLE_ROW, Table, TableCard,
};
use leptos::prelude::*;
use leptos_router::hooks::use_query_map;

fn kind_value(kind: vpb::VatPeriodKind) -> &'static str {
    match kind {
        vpb::VatPeriodKind::Monthly => "monthly",
        vpb::VatPeriodKind::Yearly => "yearly",
        vpb::VatPeriodKind::NotRegistered => "not_registered",
        _ => "quarterly",
    }
}

fn kind_of(value: &str) -> vpb::VatPeriodKind {
    match value {
        "monthly" => vpb::VatPeriodKind::Monthly,
        "yearly" => vpb::VatPeriodKind::Yearly,
        "not_registered" => vpb::VatPeriodKind::NotRegistered,
        _ => vpb::VatPeriodKind::Quarterly,
    }
}

pub fn vat_status_label(status: vpb::VatStatus) -> &'static str {
    match status {
        vpb::VatStatus::InProgress => "Pågår",
        vpb::VatStatus::ToSubmit => "Att lämna",
        vpb::VatStatus::Submitted => "Inlämnad",
        vpb::VatStatus::Changed => "Ändrad",
        vpb::VatStatus::Unspecified => "",
    }
}

pub fn vat_status_variant(status: vpb::VatStatus) -> BadgeVariant {
    match status {
        vpb::VatStatus::Changed => BadgeVariant::Destructive,
        vpb::VatStatus::Submitted => BadgeVariant::Secondary,
        _ => BadgeVariant::Outline,
    }
}

pub fn box_amount(kr: i64, vat_box: u32) -> String {
    // Whole kronor, grouped as everywhere else.
    let grouped = amount(kr * 100).trim_end_matches(",00").to_owned();
    match (kr, vat_box) {
        (0, _) => "–".into(),
        // The form prints box 48 with a minus: it is deducted.
        (_, 48) => format!("−{grouped}"),
        _ => grouped,
    }
}

#[component]
pub fn Vat() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let query = use_query_map();
    let error = RwSignal::new(None::<String>);
    let preferred = query.read_untracked().get("fy").unwrap_or_default();
    let (years, year) = use_fiscal_years(preferred, error);
    keep_year_in_url("/vat".into(), year);
    // The year's answer and the (company, year) it is for.
    let loaded = RwSignal::new(None::<(String, String, vpb::ListVatReturnsResponse)>);
    let kind = RwSignal::new(String::new());
    // A SetVatPeriod call is in flight.
    let saving = RwSignal::new(false);

    let load = move || {
        let (company_id, start) = (companies.active.get_untracked(), year.get_untracked());
        if company_id.is_empty() || start.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = vat_api()
                .list_vat_returns(vpb::ListVatReturnsRequest {
                    company_id: company_id.clone(),
                    fiscal_year_start: start.clone(),
                })
                .await;
            if company_id != companies.active.get_untracked() || start != year.get_untracked() {
                return;
            }
            match result {
                Ok(response) => {
                    let response = response.into_inner();
                    kind.set(kind_value(response.kind()).into());
                    loaded.set(Some((company_id, start, response)));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        year.track();
        loaded.set(None);
        load();
    });
    // A change of the select (not the value loaded into it) is saved.
    Effect::new(move |_| {
        let chosen = kind.get();
        let Some((company_id, start, response)) = loaded.get_untracked() else {
            return;
        };
        if chosen.is_empty() || chosen == kind_value(response.kind()) {
            return;
        }
        error.set(None);
        saving.set(true);
        spawn_local(async move {
            let request = vpb::SetVatPeriodRequest {
                company_id,
                fiscal_year_start: start,
                kind: kind_of(&chosen) as i32,
            };
            let result = vat_api().set_vat_period(request).await;
            saving.set(false);
            match result {
                Ok(_) => load(),
                Err(status) => {
                    error.set(Some(describe(&status)));
                    load();
                }
            }
        });
    });

    view! {
        <div class="grid gap-6">
            <PageHeader title="Moms">
                <FiscalYearSelect years=years year=year />
            </PageHeader>
            <ErrorAlert message=error />
            <Card title="Redovisningsperiod" narrow=true>
                <Select label="Redovisningsperiod" id="vat_period_kind" hide_label=true value=kind disabled=Signal::derive(move || saving.get() || loaded.get().is_none_or(|(_, _, r)| r.locked))>
                    <option class=SELECT_OPTION value="monthly">"Månad"</option>
                    <option class=SELECT_OPTION value="quarterly">"Kvartal"</option>
                    <option class=SELECT_OPTION value="yearly">"Helår"</option>
                    <option class=SELECT_OPTION value="not_registered">"Ej momsregistrerad"</option>
                </Select>
                {move || loaded.get().filter(|(_, _, r)| r.locked).map(|_| view! {
                    <p class="mt-2 text-xs/relaxed text-muted-foreground">"Perioden kan inte ändras när en deklaration för året är inlämnad."</p>
                })}
            </Card>
            <TableCard><Table>
                <thead class=TABLE_HEAD>
                    <tr class=TABLE_ROW>
                        <th class=TABLE_HEADER_CELL>"Period"</th>
                        <th class=TABLE_HEADER_CELL>"Status"</th>
                        <th class=TABLE_HEADER_CELL>"Deklarationsdag"</th>
                        <th class=format!("{TABLE_HEADER_CELL} text-right")>"Att betala/få tillbaka"</th>
                    </tr>
                </thead>
                <tbody class=TABLE_BODY>
                    {move || loaded.get().map(|(_, _, r)| r.periods.into_iter().map(|p| {
                        let status = p.status();
                        view! {
                            <tr class=TABLE_ROW>
                                <td class=TABLE_CELL><a class="underline-offset-4 hover:underline" href=format!("/vat/{}", p.period)>{p.label.clone()}</a></td>
                                <td class=TABLE_CELL><Badge variant=vat_status_variant(status)>{vat_status_label(status)}</Badge></td>
                                <td class=TABLE_CELL>{if p.due_date.is_empty() { "Se Skatteverket".to_owned() } else { p.due_date.clone() }}</td>
                                <td class=TABLE_AMOUNT_CELL>{whole_kronor(p.vat_due * 100)}</td>
                            </tr>
                        }
                    }).collect_view())}
                </tbody>
            </Table></TableCard>
        </div>
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn statuses_and_amounts_read_as_on_the_form() {
        assert_eq!(vat_status_label(vpb::VatStatus::InProgress), "Pågår");
        assert_eq!(vat_status_label(vpb::VatStatus::ToSubmit), "Att lämna");
        assert_eq!(vat_status_label(vpb::VatStatus::Submitted), "Inlämnad");
        assert_eq!(vat_status_label(vpb::VatStatus::Changed), "Ändrad");
        assert_eq!(box_amount(412_300, 5), "412\u{a0}300");
        assert_eq!(box_amount(24_610, 48), "−24\u{a0}610");
        assert_eq!(box_amount(0, 10), "–");
        assert_eq!(box_amount(-1_500, 42), "-1\u{a0}500");
    }

    #[test]
    fn a_changed_period_stands_out_and_a_submitted_one_recedes() {
        assert!(vat_status_variant(vpb::VatStatus::Changed) == BadgeVariant::Destructive);
        assert!(vat_status_variant(vpb::VatStatus::Submitted) == BadgeVariant::Secondary);
        for status in [vpb::VatStatus::InProgress, vpb::VatStatus::ToSubmit] {
            assert!(vat_status_variant(status) == BadgeVariant::Outline);
        }
    }
}
