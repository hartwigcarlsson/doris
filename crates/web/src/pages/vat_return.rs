//! Momsdeklaration for one period, drawn as SKV 4700: the boxes with the
//! accounts behind them, the file for Skatteverket and marking it submitted.

use crate::active_company::Companies;
use crate::api::{vat_api, vpb};
use crate::attachments::save_as;
use crate::errors::describe;
use crate::format::amount;
use crate::pages::vat::{box_amount, vat_status_label, vat_status_variant};
use crate::task::spawn_local;
use crate::ui::{Badge, Button, ErrorAlert, PageHeader, Variant};
use crate::vat_form::{SECTIONS, Section};
use leptos::prelude::*;
use leptos_router::hooks::use_params_map;

#[component]
pub fn VatReturnPage() -> impl IntoView {
    let companies = expect_context::<Companies>();
    let period = use_params_map()
        .read_untracked()
        .get("period")
        .unwrap_or_default();
    let period = StoredValue::new(period);
    let error = RwSignal::new(None::<String>);
    let declaration = RwSignal::new(None::<(String, vpb::VatReturn)>);
    let downloaded = RwSignal::new(None::<String>);
    let confirming = RwSignal::new(false);
    let marking = RwSignal::new(false);
    let booked = RwSignal::new(None::<String>);

    let load = move || {
        let company_id = companies.active.get_untracked();
        if company_id.is_empty() {
            return;
        }
        spawn_local(async move {
            let result = vat_api()
                .get_vat_return(vpb::VatReturnRef {
                    company_id: company_id.clone(),
                    period: period.get_value(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(r) => {
                    downloaded.set(None);
                    declaration.set(Some((company_id, r.into_inner())));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    Effect::new(move |_| {
        companies.active.track();
        declaration.set(None);
        downloaded.set(None);
        confirming.set(false);
        booked.set(None);
        error.set(None);
        load();
    });

    let download = move |_| {
        error.set(None);
        let Some((company_id, _)) = declaration.get_untracked() else {
            return;
        };
        spawn_local(async move {
            let result = vat_api()
                .export_vat_file(vpb::VatReturnRef {
                    company_id: company_id.clone(),
                    period: period.get_value(),
                })
                .await;
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(file) => {
                    let file = file.into_inner();
                    save_as(&file.file_name, "application/xml", &file.content);
                    downloaded.set(Some(file.fingerprint));
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };
    let submit = move |_| {
        if marking.get_untracked() {
            return;
        }
        let Some((company_id, shown)) = declaration.get_untracked() else {
            return;
        };
        marking.set(true);
        error.set(None);
        let fingerprint = downloaded.get_untracked().unwrap_or(shown.fingerprint);
        spawn_local(async move {
            let result = vat_api()
                .mark_vat_return_submitted(vpb::MarkVatReturnSubmittedRequest {
                    company_id: company_id.clone(),
                    period: period.get_value(),
                    fingerprint,
                })
                .await;
            marking.set(false);
            if company_id != companies.active.get_untracked() {
                return;
            }
            match result {
                Ok(r) => {
                    let r = r.into_inner();
                    confirming.set(false);
                    booked.set(Some(if r.voucher_number == 0 {
                        "Inlämnad. Det fanns inget att bokföra.".into()
                    } else {
                        format!(
                            "Inlämnad. Momsavräkningen bokfördes som verifikation {}.",
                            r.voucher_number
                        )
                    }));
                    load();
                }
                Err(status) => error.set(Some(describe(&status))),
            }
        });
    };

    view! {
        <div class="grid gap-6">
            {move || {
                let title = declaration.get().and_then(|(_, d)| d.summary).map_or("Momsdeklaration".to_owned(), |s| format!("Momsdeklaration {}", s.label));
                view! {
                    <PageHeader title=title>
                        <Show when=move || declaration.get().and_then(|(_, d)| d.summary).is_some_and(|s| matches!(s.status(), vpb::VatStatus::ToSubmit | vpb::VatStatus::Changed))>
                            <Button variant=Variant::Outline kind="button" on:click=download>"Ladda ner fil"</Button>
                            <Button kind="button" on:click=move |_| confirming.set(true)>"Markera inlämnad…"</Button>
                        </Show>
                    </PageHeader>
                }
            }}
            <ErrorAlert message=error />
            {move || booked.get().map(|text| view! { <p role="status" class="text-xs/relaxed">{text}</p> })}
            <Show when=move || confirming.get()>
                {move || declaration.get().and_then(|(_, d)| d.summary).map(|s| view! {
                    <div class="flex flex-wrap items-center gap-2 text-xs/relaxed">
                        <span>{format!("Markera som inlämnad när filen är uppladdad hos Skatteverket. Doris bokför momsavräkningen daterad {}.", s.end)}</span>
                        <Button kind="button" disabled=Signal::derive(move || marking.get()) on:click=submit>"Bekräfta"</Button>
                        <Button variant=Variant::Ghost kind="button" on:click=move |_| confirming.set(false)>"Avbryt"</Button>
                    </div>
                })}
            </Show>
            {move || declaration.get().map(|(_, d)| view! { <Declaration declaration=d /> })}
        </div>
    }
}

#[component]
fn Declaration(declaration: vpb::VatReturn) -> impl IntoView {
    let summary = declaration.summary.clone().unwrap_or_default();
    let status = summary.status();
    let rounding = summary.vat_due * 100 - declaration.booked_vat;
    let boxes = StoredValue::new(declaration.boxes.clone());
    let column = move |right: bool| {
        SECTIONS
            .iter()
            .filter(move |s| s.right == right)
            .map(move |s| {
                view! {
                    <FormSection section=s boxes=boxes vat_due=summary.vat_due />
                }
            })
            .collect_view()
    };
    view! {
        <div class="grid gap-3">
            <div class="flex flex-wrap items-center gap-2 text-xs/relaxed">
                <Badge variant=vat_status_variant(status)>{vat_status_label(status)}</Badge>
                <span class="text-muted-foreground">{format!("Organisationsnummer {} · Momsregistreringsnummer {} · Deklarationsdag {} · Period i filen {}",
                    declaration.org_nr, declaration.vat_number,
                    if summary.due_date.is_empty() { "se Skatteverket".to_owned() } else { summary.due_date.clone() },
                    summary.period)}</span>
            </div>
            <p class="text-xs/relaxed text-muted-foreground">"Ange endast kronor, ej ören."</p>
            <div class="grid gap-3 md:grid-cols-2">
                <div class="grid content-start gap-3">{column(false)}</div>
                <div class="grid content-start gap-3">
                    {column(true)}
                    <p class="text-xs/relaxed text-muted-foreground">
                        {format!("Bokförd moms {} · Öresavrundning (3740) {}", amount(declaration.booked_vat), amount(rounding))}
                    </p>
                </div>
            </div>
            {(!declaration.submissions.is_empty()).then(|| view! {
                <section class="grid gap-1 text-xs/relaxed">
                    <h2 class="text-sm font-medium">"Inlämningar"</h2>
                    {declaration.submissions.iter().map(|s| {
                        let voucher = if s.voucher_number == 0 { "ingen verifikation".to_owned() } else { format!("ver. {}", s.voucher_number) };
                        let corrected = if s.corrected { " · Rättad" } else { "" };
                        view! { <p>{format!("{} · {} · {voucher}{corrected}", s.submitted_at, s.submitted_by_name)}</p> }
                    }).collect_view()}
                </section>
            })}
        </div>
    }
}

#[component]
fn FormSection(
    section: &'static Section,
    boxes: StoredValue<Vec<vpb::VatBoxAmount>>,
    vat_due: i64,
) -> impl IntoView {
    view! {
        <section class="overflow-hidden rounded-lg bg-card ring-1 ring-foreground/10">
            <h2 class="border-b bg-muted px-3 py-2 text-xs font-medium">{format!("{}. {}", section.letter, section.title)}</h2>
            {section.rows.iter().map(|row| {
                let n = row.vat_box;
                let found = boxes.with_value(|b| b.iter().find(|b| b.r#box == n).cloned());
                let kr = if n == 49 { vat_due } else { found.as_ref().map_or(0, |b| b.amount) };
                let open = RwSignal::new(false);
                let accounts = found.map(|b| b.accounts).unwrap_or_default();
                let has_accounts = !accounts.is_empty();
                let value_class = if n == 49 { "rounded-md px-2 py-0.5 text-right tabular-nums ring-1 ring-primary" } else { "rounded-md px-2 py-0.5 text-right tabular-nums ring-1 ring-input" };
                view! {
                    <div class="border-t first:border-t-0">
                        <button type="button" class="grid w-full grid-cols-[1fr_2rem_7rem] items-center gap-2 px-3 py-1.5 text-left text-xs/relaxed aria-expanded:bg-accent"
                            aria-expanded=move || open.get().to_string()
                            disabled=!has_accounts
                            on:click=move |_| open.update(|o| *o = !*o)>
                            <span>{row.label}</span>
                            <span class="text-right font-medium text-muted-foreground">{format!("{n:02}")}</span>
                            <span class=value_class>{box_amount(kr, n)}</span>
                        </button>
                        <Show when=move || open.get()>
                            <div class="grid gap-0.5 bg-accent px-6 pb-2 text-xs/relaxed text-muted-foreground">
                                {accounts.iter().map(|a| view! {
                                    <div class="flex justify-between gap-4"><span>{format!("{} {}", a.number, a.name)}</span><span class="tabular-nums">{amount(a.amount)}</span></div>
                                }).collect_view()}
                            </div>
                        </Show>
                    </div>
                }
            }).collect_view()}
        </section>
    }
}
